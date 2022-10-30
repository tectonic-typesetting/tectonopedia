// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! "Pass 1"
//!
//! - Have to launch TeX in subprocesses because the engine can't be multithreaded
//! - Use a threadpool to manage that
//! - Subprocess stderr is passed straight on through for error reporing
//! - Subprocess stdout is parsed for information transfer

use clap::Args;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
};
use tectonic::{
    config::PersistentConfig,
    driver::{PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
    unstable_opts::UnstableOptions,
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_error, tt_warning, StatusBackend};
use walkdir::DirEntry;

use crate::worker_status::WorkerStatusBackend;

#[derive(Debug)]
pub enum FirstPassError<T> {
    /// Some kind of environmental error not specific to this particular input.
    /// We should abort the whole build because other jobs are probably going to
    /// fail too.
    General(T),

    /// An error specific to this input. We'll fail this input, but keep on
    /// going overall to report as many problems as we can.
    Specific(T),
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

pub fn build_one_input(self_path: PathBuf, entry: DirEntry) -> Result<(), FirstPassError<()>> {
    // This function is run in a fresh thread, so it needs to create its own
    // status backend if it wants to report any information (because our status
    // system is not thread-safe). It also needs to do that to provide context
    // about the origin of any messages. It should fully report out any errors
    // that it encounters.
    let mut status =
        Box::new(WorkerStatusBackend::new(entry.path().display())) as Box<dyn StatusBackend>;

    let mut cmd = Command::new(&self_path);
    cmd.arg("first-pass-impl")
        .arg(entry.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tt_error!(status, "failed to relaunch self as TeX worker"; e.into());
            return Err(FirstPassError::General(()));
        }
    };

    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut error_type = FirstPassError::Specific(());

    for line in stdout.lines() {
        match line {
            Ok(line) => {
                if let Some(rest) = line.strip_prefix("pedia:") {
                    match rest {
                        "general-error" => {
                            error_type = FirstPassError::General(());
                        }
                        _ => {
                            tt_warning!(status.as_mut(), "unrecognized stdout message: {}", line);
                        }
                    }
                } else {
                    tt_warning!(status.as_mut(), "unexpected stdout content: {}", line);
                }
            }

            Err(e) => {
                tt_warning!(status.as_mut(), "error reading worker stdout"; e.into());
            }
        }
    }

    let ec = match child.wait() {
        Ok(c) => c,
        Err(e) => {
            tt_error!(status, "failed to wait() for TeX worker"; e.into());
            return Err(error_type);
        }
    };

    match (ec.success(), &error_type) {
        (true, FirstPassError::Specific(_)) => Ok(()), // <= the default
        (true, FirstPassError::General(_)) => {
            tt_warning!(
                status.as_mut(),
                "TeX worker had a successful exit code but reported failure"
            );
            Err(error_type)
        }
        (false, _) => Err(error_type),
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
        match self.inner(status) {
            Ok(_) => Ok(()),

            Err(FirstPassError::General(e)) => {
                println!("pedia:general-error");
                Err(e)
            }

            Err(FirstPassError::Specific(e)) => Err(e),
        }
    }

    fn inner(&self, status: &mut dyn StatusBackend) -> Result<(), FirstPassError<Error>> {
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
