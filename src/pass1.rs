// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! "Pass 1"

use clap::Args;
use std::process::Command;
use tectonic::{
    config::PersistentConfig,
    driver::{PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
    unstable_opts::UnstableOptions,
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_warning, StatusBackend};
use walkdir::DirEntry;

use crate::{
    gtry, ogtry, ostry,
    texworker::{WorkerDriver, WorkerError, WorkerResultExt},
};

#[derive(Debug, Default)]
pub struct Pass1Driver {}

impl WorkerDriver for Pass1Driver {
    type Item = ();

    fn init_command(&self, cmd: &mut Command, entry: &DirEntry) {
        cmd.arg("first-pass-impl").arg(entry.path());
    }

    fn process_output_record(&mut self, record: &str, status: &mut dyn StatusBackend) {
        tt_warning!(status, "unrecognized pass1 stdout record: {}", record);
    }

    fn finish(self) -> () {
        ()
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
            .filesystem_root(root)
            .unstables(unstables)
            .format_cache_path(ogtry!(config.format_cache_path()))
            .do_not_write_output_files()
            .pass(PassSetting::Tex);

        let mut sess = ogtry!(sess.create(status));

        // Print more details in the error case here?
        ostry!(sess.run(status));

        Ok(())
    }
}
