// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! "Pass 1"

use clap::Args;
use std::{
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufRead, BufReader, Cursor, Write},
    process::{ChildStdin, Command},
};
use tectonic::{
    config::PersistentConfig,
    driver::{OutputFormat, PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
    unstable_opts::UnstableOptions,
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_engine_spx2html::AssetSpecification;
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_warning, StatusBackend};

use crate::{
    cache::OpCacheData,
    gtry,
    index::{IndexCollection, IndexId},
    metadata::Metadatum,
    ogtry,
    operation::{DigestData, RuntimeEntityIdent},
    ostry, stry,
    tex_pass::{TexProcessor, WorkerDriver, WorkerError, WorkerResultExt},
    InputId,
};

#[derive(Debug)]
pub struct Pass2Processor {
    assets: AssetSpecification,
    indices: IndexCollection,
    inputs_index_id: IndexId,
    input_id: Option<InputId>,
    entrypoints_file: File,
    n_outputs: usize,
}

impl TexProcessor for Pass2Processor {
    type Worker = Pass2Driver;

    fn make_worker(
        &mut self,
        input: RuntimeEntityIdent,
        indices: &mut IndexCollection,
    ) -> Result<Self::Worker, WorkerError<Error>> {
        let rrtex = self
            .indices
            .get_resolved_reference_tex(self.input_id.unwrap());
        Ok(Pass2Driver::new(rrtex, self.assets.clone()))
    }

    fn accumulate_output(&mut self, _item: ()) {}
}

impl Pass2Processor {
    pub fn new(
        assets: AssetSpecification,
        indices: IndexCollection,
        entrypoints_file: File,
    ) -> Self {
        let inputs_index_id = indices.get_index("inputs").unwrap();

        Pass2Processor {
            assets,
            indices,
            inputs_index_id,
            input_id: None,
            entrypoints_file,
            n_outputs: 0,
        }
    }

    fn process_item_inner(&mut self, _id: InputId, item: Pass2Driver) -> Result<()> {
        for line in item.metadata_lines {
            match Metadatum::parse(&line)? {
                Metadatum::Output(path) => {
                    writeln!(self.entrypoints_file, "<a href=\"{}\"></a>", path)?;
                    self.n_outputs += 1;
                }

                _ => {}
            }
        }

        Ok(())
    }

    pub fn n_outputs(&self) -> usize {
        self.n_outputs
    }
}

#[derive(Debug)]
pub struct Pass2Driver {
    resolved_ref_tex: String,
    assets: AssetSpecification,
    metadata_lines: Vec<String>,
}

impl Pass2Driver {
    pub fn new(resolved_ref_tex: String, assets: AssetSpecification) -> Self {
        Pass2Driver {
            resolved_ref_tex,
            assets,
            metadata_lines: Default::default(),
        }
    }
}

impl WorkerDriver for Pass2Driver {
    type Item = ();

    fn operation_ident(&self) -> DigestData {
        unreachable!()
    }

    fn init_command(&self, cmd: &mut Command, task_num: usize) {
        unreachable!();
        //cmd.arg("second-pass-impl").arg(path);

        if task_num == 0 {
            cmd.arg("--first");
        }
    }

    fn send_stdin(&self, stdin: &mut ChildStdin) -> Result<()> {
        writeln!(stdin, "{}\n---", self.resolved_ref_tex)?;
        self.assets.save(stdin).map_err(|e| e.into())
    }

    fn process_output_record(&mut self, record: &str, status: &mut dyn StatusBackend) {
        if let Some(rest) = record.strip_prefix("meta ") {
            self.metadata_lines.push(rest.to_owned());
        } else {
            tt_warning!(status, "unrecognized pass2 stdout record: {}", record);
        }
    }

    fn finish(self) -> Result<(OpCacheData, ()), WorkerError<Error>> {
        unreachable!();
        //self
    }
}

#[derive(Args, Debug)]
pub struct SecondPassImplArgs {
    /// The path of the TeX file to compile
    #[arg()]
    pub tex_path: String,

    /// If this is the first TeX build in the session.
    #[arg(long)]
    pub first: bool,
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
        out_dir.push("build");
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
        sess.primary_input_buffer(&input.as_bytes())
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
            .pass(PassSetting::Default);

        if !self.first {
            // For the first output, we leave the default configuration to emit
            // the assets. For all other outputs, we want Tectonic to emit
            // the templated HTML outputs, but not the assets (font files, etc.).
            sess.html_emit_assets(false);
        }

        let mut sess = ogtry!(sess.create(status));

        // Print more details in the error case here?
        ostry!(sess.run(status));

        // Print out the `pedia.txt` metadata file

        let mut files = sess.into_file_data();

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
