// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Running the `yarn` tool.

use std::process::{Child, Command};
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

/// Run the `yarn index` command.
///
/// This isn't plugged into the incremental build system (for now?). We just run
/// the command.
pub fn yarn_index(status: &mut dyn StatusBackend) -> Result<()> {
    do_yarn("index", status)
}

/// Run the `yarn build` command.
pub fn yarn_build(status: &mut dyn StatusBackend) -> Result<()> {
    do_yarn("build", status)
}

/// Launch a `yarn serve` command.
pub fn yarn_serve() -> Result<Child> {
    let mut cmd = Command::new("yarn");
    cmd.arg("serve");

    let child = atry!(
        cmd.spawn();
        ["failed to spawn `yarn serve` command"]
    );

    Ok(child)
}
