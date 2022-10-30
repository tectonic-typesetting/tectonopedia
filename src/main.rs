// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use clap::{Args, Parser, Subcommand};
use std::process::Command;
use tectonic::status::{termcolor::TermcolorStatusBackend, ChatterLevel, StatusBackend};
use tectonic_errors::prelude::*;
use walkdir::{DirEntry, WalkDir};

mod config;
mod pass1;

fn main() {
    let args = ToplevelArgs::parse();

    let mut status =
        Box::new(TermcolorStatusBackend::new(ChatterLevel::Normal)) as Box<dyn StatusBackend>;

    if let Err(e) = args.exec(status.as_mut()) {
        status.report_error(&e);
        std::process::exit(1)
    }
}

#[derive(Debug, Parser)]
struct ToplevelArgs {
    #[command(subcommand)]
    action: Action,
}

impl ToplevelArgs {
    fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        match self.action {
            Action::Build(a) => a.exec(),
            Action::FirstPassImpl(a) => a.exec(status),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Action {
    Build(BuildArgs),
    FirstPassImpl(pass1::FirstPassImplArgs),
}

#[derive(Args, Debug)]
struct BuildArgs {
    #[arg(long)]
    sample: Option<String>,
}

impl BuildArgs {
    fn exec(self) -> Result<()> {
        let self_path = atry!(
            std::env::current_exe();
            ["cannot obtain the path to the current executable"]
        );

        for entry in WalkDir::new("txt").into_iter().filter_entry(is_tex_or_dir) {
            let entry = entry?;

            if entry.file_type().is_dir() {
                continue;
            }

            let mut cmd = Command::new(&self_path);
            cmd.arg("first-pass-impl");
            cmd.arg(entry.path());

            let s = atry!(
                cmd.status();
                ["failed to relaunch self as subcommand"]
            );

            ensure!(s.success(), "self-subcommand failed");
        }

        Ok(())
    }
}

fn is_tex_or_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        || entry
            .file_name()
            .to_str()
            .map(|s| s.ends_with(".tex"))
            .unwrap_or(false)
}
