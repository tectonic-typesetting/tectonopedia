// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! "Pass 1"

use clap::Args;
use digest::Digest;
use std::{
    io::{BufRead, BufReader, Cursor, Write},
    path::PathBuf,
    process::{ChildStdin, Command},
};
use tectonic::{
    config::PersistentConfig,
    driver::{OutputFormat, PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
    unstable_opts::UnstableOptions,
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_warning, StatusBackend};

use crate::{
    cache::OpCacheData,
    gtry,
    index::{IndexCollection, IndexId},
    //index::{EntryText, IndexCollection, IndexId, IndexRef},
    //metadata::Metadatum,
    ogtry,
    operation::{DigestComputer, DigestData, OpOutputStream, RuntimeEntityIdent},
    ostry,
    stry,
    tex_pass::{TexOperation, TexProcessor, WorkerDriver, WorkerError, WorkerResultExt},
};

/// This type manages the execution of the set of pass-1 TeX jobs.
#[derive(Debug)]
pub struct Pass1Processor {
    inputs_index_id: IndexId,
    asset_files: Vec<RuntimeEntityIdent>,
    metadata_files: Vec<RuntimeEntityIdent>,
}

impl TexProcessor for Pass1Processor {
    /// This type is sent to a worker thread to drive an actual TeX process and
    /// return any results that we care about at runtime.
    type Worker = Pass1Driver;

    fn make_op_info(
        &mut self,
        input: RuntimeEntityIdent,
        indices: &mut IndexCollection,
    ) -> Pass1OpInfo {
        // Generate the ID of this operation
        let mut dc = DigestComputer::default();
        dc.update("pass1_v2");
        input.update_digest(&mut dc, indices);
        let opid = dc.finalize();

        // Figure out the output idents.

        let stripped = {
            let input_relpath = indices.relpath_for_tex_source(input).unwrap();
            input_relpath
                .strip_suffix(".tex")
                .unwrap_or(input_relpath)
                .to_owned()
        };

        let assets_id =
            RuntimeEntityIdent::new_other_file(&format!("cache/pass1/{stripped}.assets"), indices);

        let metadata_id =
            RuntimeEntityIdent::new_other_file(&format!("cache/pass1/{stripped}.meta"), indices);

        Pass1OpInfo {
            opid,
            input_id: input,
            assets_id,
            metadata_id,
        }
    }

    fn make_worker(
        &mut self,
        opinfo: Pass1OpInfo,
        indices: &mut IndexCollection,
    ) -> Result<Self::Worker, WorkerError<Error>> {
        Pass1Driver::new(opinfo, indices)
    }

    fn accumulate_output(&mut self, opinfo: Pass1OpInfo) {
        self.asset_files.push(opinfo.assets_id);
        self.metadata_files.push(opinfo.metadata_id);
    }
}

#[derive(Debug)]
pub struct Pass1OpInfo {
    opid: DigestData,
    input_id: RuntimeEntityIdent,
    assets_id: RuntimeEntityIdent,
    metadata_id: RuntimeEntityIdent,
}

impl TexOperation for Pass1OpInfo {
    fn operation_ident(&self) -> DigestData {
        self.opid.clone()
    }
}

impl Pass1Processor {
    pub fn new(indices: &IndexCollection) -> Self {
        let inputs_index_id = indices.get_index("inputs").unwrap();

        Pass1Processor {
            asset_files: Default::default(),
            metadata_files: Default::default(),
            inputs_index_id,
        }
    }

    pub fn unpack(self) -> (Vec<RuntimeEntityIdent>, Vec<RuntimeEntityIdent>) {
        (self.asset_files, self.metadata_files)
    }

    #[cfg(OLD)]
    fn process_item_inner(
        &mut self,
        _id: InputId,
        item: Pass1Driver,
        status: &mut dyn StatusBackend,
    ) -> Result<impl IntoIterator<Item = IndexRef>> {
        atry!(
            self.assets.add_from_saved(item.assets.as_bytes());
            ["failed to import assets data"]
        );

        // Process the metadata. We coalesce index references here.

        let outputs_id = self.indices.get_index("outputs").unwrap();
        let mut cur_output = None;
        let mut index_refs = HashMap::new();

        for line in item.metadata_lines {
            match Metadatum::parse(&line)? {
                Metadatum::Output(path) => {
                    // TODO: make sure there are no redundant outputs
                    cur_output = Some(self.indices.reference_by_id(outputs_id, path));
                }

                Metadatum::IndexDef {
                    index,
                    entry,
                    fragment,
                } => {
                    if let Err(e) = self.indices.reference(index, entry) {
                        tt_warning!(status, "couldn't define entry `{}` in index `{}`", entry, index; e);
                        continue;
                    }

                    let co = match cur_output.as_ref() {
                        Some(o) => *o,
                        None => {
                            tt_warning!(status, "attempt to define entry `{}` in index `{}` before an output has been specified", entry, index);
                            continue;
                        }
                    };

                    let loc = self.indices.make_location_by_id(co, fragment);

                    if let Err(e) = self.indices.define_loc(index, entry, loc) {
                        // The error here will contain the contextual information.
                        tt_warning!(status, "couldn't define an index entry"; e);
                    }
                }

                Metadatum::IndexRef {
                    index,
                    entry,
                    flags,
                } => {
                    let ie = match self.indices.reference_to_entry(index, entry) {
                        Ok(ie) => ie,

                        Err(e) => {
                            tt_warning!(status, "couldn't reference entry `{}` in index `{}`", entry, index; e);
                            continue;
                        }
                    };

                    let cur_flags = index_refs.entry(ie).or_default();
                    *cur_flags |= flags;
                }

                Metadatum::IndexText {
                    index,
                    entry,
                    tex,
                    plain,
                } => {
                    if let Err(e) = self.indices.reference(index, entry) {
                        tt_warning!(status, "couldn't define entry `{}` in index `{}`", entry, index; e);
                        continue;
                    }

                    let text = EntryText {
                        tex: tex.to_owned(),
                        plain: plain.to_owned(),
                    };

                    if let Err(e) = self.indices.define_text(index, entry, text) {
                        // The error here will contain the contextual information.
                        tt_warning!(status, "couldn't define the text of an index entry"; e);
                    }
                }
            }
        }

        Ok(index_refs
            .into_iter()
            .map(|((index, entry), flags)| IndexRef {
                index,
                entry,
                flags,
            }))
    }
}

#[derive(Debug)]
pub struct Pass1Driver {
    opinfo: Pass1OpInfo,
    input_path: PathBuf,
    cache_data: OpCacheData,
    assets: OpOutputStream,
    metadata: OpOutputStream,
    n_errors: usize,
}

impl Pass1Driver {
    fn new(opinfo: Pass1OpInfo, indices: &mut IndexCollection) -> Result<Self, WorkerError<Error>> {
        let input_path = indices.path_for_runtime_ident(opinfo.input_id).unwrap();

        let mut cache_data = OpCacheData::new(opinfo.opid);
        cache_data.add_input(opinfo.input_id);

        // We'll add the outputs to the cache data at the end of the operation,
        // so that we can give the cache a hint about their final size and
        // digest.

        let assets = stry!(OpOutputStream::new(opinfo.assets_id, indices));
        let mut metadata = stry!(OpOutputStream::new(opinfo.metadata_id, indices));

        // Log the path of the input file so downstream processes can easily associate
        // the indexing data with it.

        stry!(writeln!(
            metadata,
            "% input {}",
            indices.relpath_for_tex_source(opinfo.input_id).unwrap()
        ));

        Ok(Pass1Driver {
            opinfo,
            input_path,
            cache_data,
            assets,
            metadata,
            n_errors: 0,
        })
    }
}

impl WorkerDriver for Pass1Driver {
    type OpInfo = Pass1OpInfo;

    fn init_command(&self, cmd: &mut Command, _task_num: usize) {
        cmd.arg("first-pass-impl").arg(&self.input_path);
    }

    fn send_stdin(&self, _stdin: &mut ChildStdin) -> Result<()> {
        Ok(())
    }

    // TODO: record additional inputs if/when they are detected

    fn process_output_record(&mut self, record: &str, status: &mut dyn StatusBackend) {
        if let Some(rest) = record.strip_prefix("assets ") {
            if let Err(e) = writeln!(&mut self.assets, "{}", rest) {
                tt_warning!(status, "error writing asset data to `{}`", self.assets.display_path(); e.into());
                self.n_errors += 1;
            }
        } else if let Some(rest) = record.strip_prefix("meta ") {
            if let Err(e) = writeln!(&mut self.metadata, "{}", rest) {
                tt_warning!(status, "error writing metadata to `{}`", self.metadata.display_path(); e.into());
                self.n_errors += 1;
            }
        } else {
            tt_warning!(status, "unrecognized pass1 stdout record: {}", record);
        }
    }

    fn finish(mut self) -> Result<(OpCacheData, Pass1OpInfo), WorkerError<Error>> {
        let (assets_entity, size) = stry!(self.assets.close());
        self.cache_data.add_output_with_value(
            assets_entity.ident,
            assets_entity.value_digest,
            size,
        );

        let (meta_entity, size) = stry!(self.metadata.close());
        self.cache_data
            .add_output_with_value(meta_entity.ident, meta_entity.value_digest, size);

        Ok((self.cache_data, self.opinfo))
    }
}

#[derive(Args, Debug)]
pub struct FirstPassImplArgs {
    /// The path of the TeX file to compile
    #[arg()]
    pub tex_path: String,
}

impl FirstPassImplArgs {
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        self.inner(status).unwrap_for_worker()
    }

    fn inner(&self, status: &mut dyn StatusBackend) -> Result<(), WorkerError<Error>> {
        let config: PersistentConfig = ogtry!(PersistentConfig::open(false));
        let security = SecuritySettings::new(SecurityStance::MaybeAllowInsecures);
        let root = gtry!(crate::config::get_root());

        let mut cls = root.clone();
        cls.push("cls");
        let unstables = UnstableOptions {
            extra_search_paths: vec![cls],
            ..UnstableOptions::default()
        };

        let input = format!(
            "\\newif\\ifpassone \
            \\passonetrue \
            \\input{{preamble}} \
            \\input{{{}}} \
            \\input{{postamble}}\n",
            self.tex_path
        );

        let mut sess = ProcessingSessionBuilder::new_with_security(security);
        sess.primary_input_buffer(&input.as_bytes())
            .tex_input_name("texput")
            .build_date(std::time::SystemTime::now())
            .bundle(ogtry!(config.default_bundle(false, status)))
            .format_name("latex")
            .output_format(OutputFormat::Html)
            .do_not_write_output_files()
            .filesystem_root(root)
            .unstables(unstables)
            .format_cache_path(ogtry!(config.format_cache_path()))
            .html_emit_files(false)
            .html_assets_spec_path("assets.json")
            .pass(PassSetting::Default);

        let mut sess = ogtry!(sess.create(status));

        // Print more details in the error case here?
        ostry!(sess.run(status));

        // Print out the assets info

        let mut files = sess.into_file_data();

        let assets = stry!(files
            .remove("assets.json")
            .ok_or_else(|| anyhow!("no `assets.json` file output")));
        let assets = BufReader::new(Cursor::new(&assets.data));

        for line in assets.lines() {
            let line = stry!(line.context("error reading line of `assets.json` output"));
            println!("pedia:assets {}", line);
        }

        // Print out the `pedia.txt` metadata file

        let assets = stry!(files
            .remove("pedia.txt")
            .ok_or_else(|| anyhow!("no `pedia.txt` file output")));
        let assets = BufReader::new(Cursor::new(&assets.data));

        for line in assets.lines() {
            let line = stry!(line.context("error reading line of `pedia.txt` output"));
            println!("pedia:meta {}", line);
        }

        Ok(())
    }
}
