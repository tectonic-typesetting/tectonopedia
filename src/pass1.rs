// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use clap::Args;
use std::ffi::OsString;
use tectonic::{
    config::PersistentConfig,
    driver::{PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;

#[derive(Args, Debug)]
pub struct FirstPassImplArgs {
    /// The path of the TeX file to compile
    #[arg()]
    tex_path: OsString,
}

enum FirstPassError {
    /// Some kind of environmental error not specific to this particular input.
    /// We should abort the whole build because other jobs are probably going to
    /// fail too.
    General(Error),

    /// An error specific to this input. We'll fail this input, but keep on
    /// going overall to report as many problems as we can.
    Specific(Error),
}

/// Try something that returns an OldError, and report a General error if it fails.
macro_rules! ogtry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: OldError = e;
                return Err(FirstPassError::General(SyncError::new(typecheck).into()));
            }
        }
    };
}

/// Try something that returns a new Error, and report a General error if it fails.
macro_rules! gtry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: Error = e.into();
                return Err(FirstPassError::General(typecheck));
            }
        }
    };
}

/// Try something that returns an OldError, and report a Specific error if it fails.
macro_rules! ostry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: OldError = e;
                return Err(FirstPassError::Specific(SyncError::new(typecheck).into()));
            }
        }
    };
}

impl FirstPassImplArgs {
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        match self.inner(status) {
            Ok(_) => Ok(()),

            Err(FirstPassError::General(e)) => {
                println!("pedia: general-error");
                Err(e)
            }

            Err(FirstPassError::Specific(e)) => {
                println!("pedia: specific-error");
                Err(e)
            }
        }
    }

    fn inner(&self, status: &mut dyn StatusBackend) -> Result<(), FirstPassError> {
        let config: PersistentConfig = ogtry!(PersistentConfig::open(false));
        let security = SecuritySettings::new(SecurityStance::MaybeAllowInsecures);
        let root = gtry!(crate::config::get_root());

        let mut cls = root.clone();
        cls.push("cls");

        let mut sess = ProcessingSessionBuilder::new_with_security(security);
        sess.primary_input_path(&self.tex_path)
            .tex_input_name("texput")
            .build_date(std::time::SystemTime::now())
            .bundle(ogtry!(config.default_bundle(false, status)))
            .format_name("latex")
            .filesystem_root(cls)
            .format_cache_path(ogtry!(config.format_cache_path()))
            .do_not_write_output_files()
            .pass(PassSetting::Tex);

        let mut sess = ogtry!(sess.create(status));

        // Print more details in the error case here?
        ostry!(sess.run(status));

        Ok(())
    }
}
