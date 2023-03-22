// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Running the `yarn` tool.

use std::process::Command;
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_note, StatusBackend};

/// Run the `yarn index` command.
///
/// This isn't plugged into the incremental build system (for now?). We just run
/// the command.
pub fn yarn_index(status: &mut dyn StatusBackend) -> Result<()> {
    let mut cmd = Command::new("yarn");
    cmd.arg("index");

    tt_note!(status, "running `yarn index`:");
    println!();

    let ec = cmd.status();
    println!();

    let ec = atry!(
        ec;
        ["failed to execute the `yarn index` subprocess"]
    );

    if !ec.success() {
        bail!("the `yarn index` subprocess exited with an error code");
    }

    Ok(())
}
