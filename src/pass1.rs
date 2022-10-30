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
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::StatusBackend;
use walkdir::DirEntry;

pub enum FirstPassError {
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

/// Try something that returns a new Error, and report a Specific error if it fails.
macro_rules! stry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: Error = e.into();
                return Err(FirstPassError::Specific(typecheck));
            }
        }
    };
}

pub fn build_one_input(self_path: PathBuf, entry: DirEntry) -> Result<(), FirstPassError> {
    let mut cmd = Command::new(&self_path);
    cmd.arg("first-pass-impl")
        .arg(entry.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped());

    let mut child = gtry!(cmd.spawn().context("failed to relaunch self as TeX worker"));
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut error_is_general = false;

    for line in stdout.lines() {
        match line {
            Ok(line) => {
                if let Some(rest) = line.strip_prefix("pedia:") {
                    match rest {
                        "general-error" => {
                            error_is_general = true;
                        }
                        _ => {
                            eprintln!(
                                "warning({}): unrecognized stdout message: {}",
                                entry.path().display(),
                                line
                            );
                        }
                    }
                } else {
                    eprintln!(
                        "warning({}): unexpected stdout content: {}",
                        entry.path().display(),
                        line
                    );
                }
            }

            Err(e) => {
                eprintln!(
                    "warning({}): error reading stdout: {}",
                    entry.path().display(),
                    e
                );
            }
        }
    }

    let status = stry!(child
        .wait()
        .context("failed to wait for TeX worker subprocess"));

    match (status.success(), error_is_general) {
        (true, false) => Ok(()),
        (true, true) => Err(FirstPassError::General(anyhow!(
            "TeX worker for {} failed (but had a successful exit code?)",
            entry.path().display()
        ))),
        (false, true) => Err(FirstPassError::General(anyhow!(
            "TeX worker for {} failed",
            entry.path().display()
        ))),
        (false, false) => Err(FirstPassError::Specific(anyhow!(
            "TeX worker for {} failed",
            entry.path().display()
        ))),
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

    fn inner(&self, status: &mut dyn StatusBackend) -> Result<(), FirstPassError> {
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
