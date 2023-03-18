// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

use clap::{Parser, Subcommand};
use tectonic::status::termcolor::TermcolorStatusBackend;
use tectonic_errors::prelude::*;
use tectonic_status_base::{ChatterLevel, StatusBackend};

mod assets;
mod build;
mod cache;
mod config;
mod entrypoint_file;
mod holey_vec;
mod index;
mod inputs;
mod metadata;
mod multivec;
mod operation;
mod pass1;
mod pass2;
mod tex_escape;
#[macro_use]
mod tex_pass;
mod worker_status;

use worker_status::WorkerStatusBackend;

use string_interner::DefaultSymbol as InputId;

fn main() {
    let args = ToplevelArgs::parse();

    let mut status = match &args.action {
        Action::FirstPassImpl(a) => {
            Box::new(WorkerStatusBackend::new(&a.tex_path)) as Box<dyn StatusBackend>
        }
        Action::SecondPassImpl(a) => {
            Box::new(WorkerStatusBackend::new(&a.tex_path)) as Box<dyn StatusBackend>
        }
        _ => Box::new(TermcolorStatusBackend::new(ChatterLevel::Normal)) as Box<dyn StatusBackend>,
    };

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
            Action::Build(a) => a.exec(status),
            Action::FirstPassImpl(a) => a.exec(status),
            Action::SecondPassImpl(a) => a.exec(status),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Action {
    Build(build::BuildArgs),
    FirstPassImpl(pass1::FirstPassImplArgs),
    SecondPassImpl(pass2::SecondPassImplArgs),
}
