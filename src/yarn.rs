// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Running the `yarn` tool.

use std::process::{Command, Stdio};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_note, StatusBackend};

/// Run a `yarn` command.
fn do_yarn(command: &str, status: &mut dyn StatusBackend) -> Result<()> {
    let mut cmd = Command::new("yarn");
    cmd.arg(command);

    tt_note!(status, "running `yarn {}`:", command);
    println!();

    let ec = cmd.status();
    println!();

    let ec = atry!(
        ec;
        ["failed to execute the `yarn {}` subprocess", command]
    );

    if !ec.success() {
        bail!(
            "the `yarn {}` subprocess exited with an error code",
            command
        );
    }

    Ok(())
}

/// Run a `yarn` command, quietly.
fn do_yarn_quiet(command: &str) -> Result<()> {
    let mut cmd = Command::new("yarn");
    cmd.arg(command).stdout(Stdio::null());

    let ec = cmd.status();

    let ec = atry!(
        ec;
        ["failed to execute the `yarn {}` subprocess", command]
    );

    if !ec.success() {
        bail!(
            "the `yarn {}` subprocess exited with an error code",
            command
        );
    }

    Ok(())
}

/// Run the `yarn index` command.
///
/// This isn't plugged into the incremental build system (for now?). We just run
/// the command.
pub fn yarn_index(terse_output: bool, status: &mut dyn StatusBackend) -> Result<()> {
    if terse_output {
        do_yarn_quiet("index")
    } else {
        do_yarn("index", status)
    }
}

/// Run the `yarn build` command.
pub fn yarn_build(status: &mut dyn StatusBackend) -> Result<()> {
    do_yarn("build", status)
}
