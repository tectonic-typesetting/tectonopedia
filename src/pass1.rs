// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! "Pass 1"

use clap::Args;
use std::{
    io::{BufRead, BufReader, Cursor},
    process::{ChildStdin, Command},
    sync::{Arc, Mutex},
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
    cache::{Cache, Pass1Cacher},
    gtry,
    index::{IndexCollection, IndexId},
    //index::{EntryText, IndexCollection, IndexId, IndexRef},
    //metadata::Metadatum,
    ogtry,
    ostry,
    stry,
    texworker::{TexReducer, WorkerDriver, WorkerError, WorkerResultExt},
    worker_status::WorkerStatusBackend,
    InputId,
};

#[derive(Debug)]
pub struct Pass1Reducer {
    assets: AssetSpecification,
    cache: Arc<Mutex<Cache>>,
    indices: IndexCollection,
    inputs_index_id: IndexId,
}

impl TexReducer for Pass1Reducer {
    type Worker = Pass1Driver;

    fn assign_input_id(&mut self, input_name: String) -> InputId {
        self.indices
            .reference_by_id(self.inputs_index_id, input_name)
    }

    fn needs_to_be_run(&mut self, id: InputId) -> Result<bool, WorkerError<Error>> {
        let input_path = self.indices.resolve_by_id(self.inputs_index_id, id);
        let mut status = WorkerStatusBackend::new(input_path);
        let mut cache = self.cache.lock().unwrap();
        Ok(stry!(cache.input_needs_pass1(input_path, &mut status)))
    }

    fn make_worker(&mut self, id: InputId) -> Result<Self::Worker, WorkerError<Error>> {
        let input_path = self.indices.resolve_by_id(self.inputs_index_id, id);
        let cache = self.cache.clone();

        let mut c = self.cache.lock().unwrap();
        let cacher = stry!(Pass1Cacher::new(&input_path, &mut c));
        Ok(Pass1Driver {
            cacher,
            cache,
            n_errors: 0,
        })
    }

    fn process_item(&mut self, id: InputId, item: Pass1Driver) -> Result<(), WorkerError<()>> {
        let input_path = self.indices.resolve_by_id(self.inputs_index_id, id);
        let mut status = WorkerStatusBackend::new(input_path);

        if let Err(e) = self.process_item_inner(id, item, &mut status) {
            tt_error!(status, "failed to process pass 1 data"; e);
            return Err(WorkerError::Specific(()));
        }

        //match self.process_item_inner(id, item, &mut status) {
        //    Ok(index_refs) => {
        //        // This function only fails if the references for the given input have
        //        // already been logged, which should never happen to us.
        //        self.indices.log_references(id, index_refs).unwrap();
        //    }
        //
        //    Err(e) => {
        //        self.indices.log_references(id, vec![]).unwrap();
        //        tt_error!(status, "failed to process pass 1 data"; e);
        //        return Err(WorkerError::Specific(()));
        //    }
        //}

        Ok(())
    }
}

impl Pass1Reducer {
    pub fn new(indices: IndexCollection, cache: Cache) -> Self {
        let inputs_index_id = indices.get_index("inputs").unwrap();

        Pass1Reducer {
            assets: Default::default(),
            cache: Arc::new(Mutex::new(cache)),
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
        _status: &mut dyn StatusBackend,
    ) -> Result<()> {
        let mut cache = item.cache.lock().unwrap();
        item.cacher.finalize(&mut cache)
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
    cacher: Pass1Cacher,
    cache: Arc<Mutex<Cache>>,
    n_errors: usize,
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
            if let Err(e) = self.cacher.assets_line(rest) {
                tt_warning!(status, "error writing asset data"; e);
                self.n_errors += 1;
            }
        } else if let Some(rest) = record.strip_prefix("meta ") {
            if let Err(e) = self.cacher.metadata_line(rest) {
                tt_warning!(status, "error writing meta data"; e);
                self.n_errors += 1;
            }
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
