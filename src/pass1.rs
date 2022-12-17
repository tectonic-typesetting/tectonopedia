// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! "Pass 1"

use clap::Args;
use std::{
    io::{BufRead, BufReader, Cursor},
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
use tectonic_status_base::{tt_error, tt_warning, StatusBackend};
use walkdir::DirEntry;

use crate::{
    gtry,
    index::{IndexCollection, IndexId},
    metadata::Metadatum,
    ogtry, ostry, stry,
    texworker::{TexReducer, WorkerDriver, WorkerError, WorkerResultExt},
    worker_status::WorkerStatusBackend,
    InputId,
};

#[derive(Debug)]
pub struct Pass1Reducer {
    assets: AssetSpecification,
    indices: IndexCollection,
    inputs_index_id: IndexId,
}

impl TexReducer for Pass1Reducer {
    type Worker = Pass1Driver;

    fn assign_input_id(&mut self, input_name: String) -> InputId {
        self.indices.define_by_id(self.inputs_index_id, input_name)
    }

    fn make_worker(&mut self) -> Self::Worker {
        Default::default()
    }

    fn process_item(&mut self, id: InputId, item: Pass1Driver) -> Result<(), WorkerError<()>> {
        let input_path = self.indices.resolve_by_id(self.inputs_index_id, id);
        let mut status = WorkerStatusBackend::new(input_path);

        if let Err(e) = self.process_item_inner(id, item, &mut status) {
            tt_error!(status, "failed to process pass 1 data"; e);
            return Err(WorkerError::Specific(()));
        }

        Ok(())
    }
}

impl Pass1Reducer {
    pub fn new(indices: IndexCollection) -> Self {
        let inputs_index_id = indices.get_index("inputs").unwrap();

        Pass1Reducer {
            assets: Default::default(),
            indices,
            inputs_index_id,
        }
    }

    pub fn unpack(self) -> (AssetSpecification, IndexCollection) {
        (self.assets, self.indices)
    }

    fn process_item_inner(
        &mut self,
        _id: InputId,
        item: Pass1Driver,
        status: &mut dyn StatusBackend,
    ) -> Result<()> {
        atry!(
            self.assets.add_from_saved(item.assets.as_bytes());
            ["failed to import assets data"]
        );

        // Process the metadata

        let mut cur_output = None;
        let outputs_id = self.indices.get_index("outputs").unwrap();

        for line in item.metadata_lines {
            match Metadatum::parse(&line)? {
                Metadatum::Output(path) => {
                    cur_output = Some(self.indices.define_by_id(outputs_id, path));
                }

                Metadatum::IndexDef { index, entry, .. } => {
                    // TODO: ignoring fragment!

                    if let Err(e) = self.indices.define(index, entry) {
                        tt_warning!(status, "couldn't define entry `{}` in index `{}`", entry, index; e);
                        continue;
                    }

                    eprintln!("defined: {} {}", index, entry);
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Pass1Driver {
    assets: String,
    metadata_lines: Vec<String>,
}

impl WorkerDriver for Pass1Driver {
    type Item = Self;

    fn init_command(&self, cmd: &mut Command, entry: &DirEntry, _task_num: usize) {
        cmd.arg("first-pass-impl").arg(entry.path());
    }

    fn send_stdin(&self, _stdin: &mut ChildStdin) -> Result<()> {
        Ok(())
    }

    fn process_output_record(&mut self, record: &str, status: &mut dyn StatusBackend) {
        if let Some(rest) = record.strip_prefix("assets ") {
            self.assets.push_str(rest);
            self.assets.push('\n');
        } else if let Some(rest) = record.strip_prefix("meta ") {
            self.metadata_lines.push(rest.to_owned());
        } else {
            tt_warning!(status, "unrecognized pass1 stdout record: {}", record);
        }
    }

    fn finish(self) -> Self::Item {
        self
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
            "\\input{{preamble}} \
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
