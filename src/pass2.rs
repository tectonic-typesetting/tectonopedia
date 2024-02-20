// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! "Pass 2"

use clap::Args;
use futures::{future, Future, FutureExt};
use sha2::Digest;
use std::{
    collections::HashSet,
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};
use string_interner::Symbol;
use tectonic::{
    config::PersistentConfig,
    driver::{OutputFormat, PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
    unstable_opts::UnstableOptions,
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_engine_spx2html::AssetSpecification;
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::StatusBackend;
use tokio::process::{ChildStdin, Command};
use tokio_util::io::SyncIoBridge;

use crate::{
    cache::{Cache, OpCacheData},
    gtry,
    index::IndexCollection,
    metadata::Metadatum,
    ogtry,
    operation::{DigestComputer, DigestData, RuntimeEntity, RuntimeEntityIdent},
    ostry,
    tex_pass::{TexOperation, TexProcessor, WorkerDriver, WorkerError, WorkerResultExt},
};

#[derive(Debug)]
pub struct Pass2Processor {
    merged_assets_id: RuntimeEntityIdent,
    assets: AssetSpecification,
    metadata_ids: Vec<RuntimeEntityIdent>,
    n_outputs_total: usize,
    n_outputs_rerun: usize,
    potential_modified_outputs: Vec<RuntimeEntity>,
}

impl Pass2Processor {
    pub fn new(
        metadata_ids: Vec<RuntimeEntityIdent>,
        merged_assets_id: RuntimeEntityIdent,
        indices: &IndexCollection,
    ) -> Result<Self> {
        // Load the merged assets info, which every TeX job will share.

        let mut assets = AssetSpecification::default();
        let assets_path = indices.path_for_runtime_ident(merged_assets_id).unwrap();

        let assets_file = atry!(
            File::open(&assets_path);
            ["failed to open input `{}`", assets_path.display()]
        );

        atry!(
            assets.add_from_saved(assets_file);
            ["failed to import assets data"]
        );

        // Ready to go

        Ok(Pass2Processor {
            merged_assets_id,
            assets,
            metadata_ids,
            n_outputs_total: 0,
            n_outputs_rerun: 0,
            potential_modified_outputs: Vec::new(),
        })
    }

    pub fn n_outputs(&self) -> (usize, usize) {
        (self.n_outputs_rerun, self.n_outputs_total)
    }

    /// Consume this object and return a vector of potentially modified outputs.
    ///
    /// The entities returned here are a collection of entity idenifiers and
    /// their digests from *before* the any build operations were run. The
    /// caller can then compute updated digests of these files and see which if
    /// any of them have actually changed. We need this functionality because we
    /// explicitly update the modification times of our output files in a big
    /// batch in `serve` mode, to try to keep things as simple as possibe for
    /// Parcel's filesystem watcher.
    pub fn into_potential_modified_outputs(self) -> Vec<RuntimeEntity> {
        self.potential_modified_outputs
    }
}

impl TexProcessor for Pass2Processor {
    type Worker = Pass2Driver;

    fn make_op_info(
        &mut self,
        input: RuntimeEntityIdent,
        cache: &mut Cache,
        indices: &mut IndexCollection,
    ) -> Result<Pass2OpInfo> {
        // The vector of metadata IDs is guaranteed to be sorted so that it can
        // be indexed by input IDs, so:

        let input_index = match input {
            RuntimeEntityIdent::TexSourceFile(s) => s.to_usize(),
            _ => unreachable!(),
        };

        let metadata_id = self.metadata_ids[input_index];

        Pass2OpInfo::new(input, metadata_id, self.merged_assets_id, cache, indices)
    }

    fn make_worker(
        &mut self,
        opinfo: Pass2OpInfo,
        indices: &mut IndexCollection,
    ) -> Result<Self::Worker, WorkerError<Error>> {
        let input_id = match opinfo.tex_input_id {
            RuntimeEntityIdent::TexSourceFile(s) => s,
            _ => unreachable!(),
        };

        let rrtex = indices.get_resolved_reference_tex(input_id);

        Ok(Pass2Driver::new(
            opinfo,
            rrtex,
            self.assets.clone(),
            indices,
        ))
    }

    fn accumulate_output(&mut self, mut item: Pass2OpInfo, was_rerun: bool) {
        self.n_outputs_total += item.html_outputs.len();

        if was_rerun {
            self.n_outputs_rerun += item.html_outputs.len();
            self.potential_modified_outputs
                .append(&mut item.html_outputs);
        }
    }
}

#[derive(Debug)]
pub struct Pass2OpInfo {
    opid: DigestData,

    // Inputs
    tex_input_id: RuntimeEntityIdent,
    merged_assets_id: RuntimeEntityIdent,
    metadata_id: RuntimeEntityIdent,
    index_ids: Vec<RuntimeEntityIdent>,

    // Outputs
    /// The entity here encodes the identities of the outputs and their digests
    /// *before* the build. After the build, we compare digests to identify any
    /// outputs that have changed, which we need to inform Parcel.js about what
    /// needs rebuilding.
    html_outputs: Vec<RuntimeEntity>,
}

impl Pass2OpInfo {
    fn new(
        input: RuntimeEntityIdent,
        metadata_id: RuntimeEntityIdent,
        merged_assets_id: RuntimeEntityIdent,
        cache: &mut Cache,
        indices: &mut IndexCollection,
    ) -> Result<Self> {
        // Construct the operation ID. We depend on a variety inputs, but the
        // operation is uniquely identified by its TeX input.

        let mut dc = DigestComputer::default();
        dc.update("pass2_v1");
        input.update_digest(&mut dc, indices);
        let opid = dc.finalize();

        // We need to load the metadata file to know what indices we need and
        // what HTML outputs will be created.
        let mut index_names = HashSet::new();
        let mut html_outputs = Vec::new();

        let meta_path = indices.path_for_runtime_ident(metadata_id).unwrap();

        let meta_file = atry!(
            File::open(&meta_path);
            ["failed to open input `{}`", meta_path.display()]
        );

        let mut meta_buf = BufReader::new(meta_file);

        // The header "% input <relpath>" line
        let mut context = String::new();
        atry!(
            meta_buf.read_line(&mut context);
            ["failed to read input `{}`", meta_path.display()]
        );

        for line in meta_buf.lines() {
            let line = atry!(
                line;
                ["failed to read input `{}`", meta_path.display()]
            );

            match Metadatum::parse(&line)? {
                Metadatum::Output(path) => {
                    let ident = RuntimeEntityIdent::new_output_file(path, indices);
                    html_outputs.push(cache.unconditional_entity(ident, indices)?);
                }

                Metadatum::IndexRef { index, .. } => {
                    index_names.insert(index.to_owned());
                }

                _ => {}
            }
        }

        let mut index_ids = Vec::new();

        for index_name in index_names.drain() {
            index_ids.push(RuntimeEntityIdent::new_other_file(
                &format!("cache/idx/{}.csv", index_name),
                indices,
            ));
        }

        Ok(Pass2OpInfo {
            opid,
            tex_input_id: input,
            merged_assets_id,
            metadata_id,
            index_ids,
            html_outputs,
        })
    }
}

impl TexOperation for Pass2OpInfo {
    fn operation_ident(&self) -> DigestData {
        self.opid
    }
}

#[derive(Debug)]
pub struct Pass2Driver {
    opinfo: Pass2OpInfo,
    input_path: PathBuf,
    cache_data: OpCacheData,
    resolved_ref_tex: String,
    assets: AssetSpecification,
}

impl Pass2Driver {
    pub fn new(
        opinfo: Pass2OpInfo,
        resolved_ref_tex: String,
        assets: AssetSpecification,
        indices: &mut IndexCollection,
    ) -> Self {
        let input_path = indices.path_for_runtime_ident(opinfo.tex_input_id).unwrap();

        let mut cache_data = OpCacheData::new(opinfo.opid);
        cache_data.add_input(opinfo.tex_input_id);
        cache_data.add_input(opinfo.metadata_id);
        cache_data.add_input(opinfo.merged_assets_id);

        for idxid in &opinfo.index_ids {
            cache_data.add_input(*idxid);
        }

        // These outputs are created by Tectonic, so we can't calculate their
        // digests as we go; so might as well register them now.
        for output in &opinfo.html_outputs {
            cache_data.add_output(output.ident);
        }

        Pass2Driver {
            opinfo,
            input_path,
            cache_data,
            resolved_ref_tex,
            assets,
        }
    }
}

impl WorkerDriver for Pass2Driver {
    type OpInfo = Pass2OpInfo;

    fn init_command(&self, cmd: &mut Command) {
        cmd.arg("second-pass-impl").arg(&self.input_path);
    }

    fn send_stdin(&self, stdin: ChildStdin) -> impl Future<Output = Result<()>> {
        // We need to bridge the async ChildStdin stream to sync-land here,
        // because that's what AssetSpecification::save() needs. Fortunately
        // tokio_util has code to do this. However, because this function is not
        // async (because we would need async trait methods) we have to buffer
        // all of the output, because (1) the bridge needs to be used in a
        // blocking thread and (2) we can't transfer a borrow of `self` to that
        // thread.

        let mut stdin = SyncIoBridge::new(stdin);
        let mut buf = Vec::new();

        writeln!(&mut buf, "{}\n---", self.resolved_ref_tex).unwrap();
        self.assets.save(&mut buf).unwrap();

        tokio::task::spawn_blocking(move || stdin.write_all(&buf)).then(|rr| match rr {
            Ok(Ok(v)) => future::ok(v),
            Ok(Err(e)) => future::err(e.into()),
            Err(e) => future::err(e.into()),
        })
    }

    // TODO: record additional inputs if/when they are detected

    fn process_output_record(&mut self, _record: &str, _status: &mut dyn StatusBackend) {}

    fn finish(self) -> Result<(OpCacheData, Pass2OpInfo), WorkerError<Error>> {
        Ok((self.cache_data, self.opinfo))
    }
}

#[derive(Args, Debug)]
pub struct SecondPassImplArgs {
    /// The path of the TeX file to compile
    #[arg()]
    pub tex_path: String,
}

impl SecondPassImplArgs {
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        self.inner(status).unwrap_for_worker()
    }

    fn inner(&self, status: &mut dyn StatusBackend) -> Result<(), WorkerError<Error>> {
        // Read the resolved-reference information from stdin.

        let rrtex = {
            let mut rrtex = String::new();
            let stdin = std::io::stdin().lock();

            for line in stdin.lines() {
                let line = gtry!(line.context("error reading line of TeX worker input"));

                if line == "---" {
                    break;
                }

                writeln!(rrtex, "{}", line).unwrap();
            }

            rrtex
        };

        // Now we can read the state information from stdin. (We can't reuse
        // the previous stdin variable because `lines()` consumes it.)

        let assets = {
            let mut assets = AssetSpecification::default();
            let stdin = std::io::stdin().lock();

            gtry!(assets
                .add_from_saved(stdin)
                .with_context(|| "unable to restore assets from stdin"));

            assets
        };

        // Now we can do all of the other TeX-launching mumbo-jumbo.

        let config: PersistentConfig = ogtry!(PersistentConfig::open(false));
        let security = SecuritySettings::new(SecurityStance::MaybeAllowInsecures);
        let root = gtry!(crate::config::get_root());

        let mut cls = root.clone();
        cls.push("cls");
        let unstables = UnstableOptions {
            extra_search_paths: vec![cls],
            ..UnstableOptions::default()
        };

        let mut out_dir = root.clone();
        out_dir.push("staging");
        gtry!(std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("cannot create output directory `{}`", out_dir.display())));

        let input = format!(
            "\\newif\\ifpassone \
            \\passonefalse \
            \\input{{preamble}} \
            {} \
            \\input{{{}}} \
            \\input{{postamble}}\n",
            rrtex, self.tex_path
        );

        let mut sess = ProcessingSessionBuilder::new_with_security(security);
        sess.primary_input_buffer(input.as_bytes())
            .tex_input_name("texput")
            .build_date(std::time::SystemTime::now())
            .bundle(ogtry!(config.default_bundle(false, status)))
            .format_name("latex")
            .output_format(OutputFormat::Html)
            .html_precomputed_assets(assets)
            .filesystem_root(&root)
            .unstables(unstables)
            .format_cache_path(ogtry!(config.format_cache_path()))
            .output_dir(&out_dir)
            .html_emit_assets(false)
            .pass(PassSetting::Default);

        let mut sess = ogtry!(sess.create(status));

        // Print more details in the error case here?
        ostry!(sess.run(status));

        // We *could* print out the `pedia.txt` metadata file, but it's not
        // currently needed.
        //
        //let mut files = sess.into_file_data();
        //
        //let assets = stry!(files
        //    .remove("pedia.txt")
        //    .ok_or_else(|| anyhow!("no `pedia.txt` file output")));
        //let assets = BufReader::new(Cursor::new(&assets.data));
        //
        //for line in assets.lines() {
        //    let line = stry!(line.context("error reading line of `pedia.txt` output"));
        //    println!("pedia:meta {}", line);
        //}

        Ok(())
    }
}
