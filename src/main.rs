// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

use clap::{Parser, Subcommand};
use tectonic::status::termcolor::TermcolorStatusBackend;
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
mod serve;
mod tex_escape;
#[macro_use]
mod tex_pass;
mod worker_status;
mod yarn;

use worker_status::WorkerStatusBackend;

use string_interner::DefaultSymbol as InputId;

fn main() {
    let args = ToplevelArgs::parse();

    let status = match &args.action {
        Action::FirstPassImpl(a) => {
            Box::new(WorkerStatusBackend::new(&a.tex_path)) as Box<dyn StatusBackend + Send>
        }
        Action::SecondPassImpl(a) => {
            Box::new(WorkerStatusBackend::new(&a.tex_path)) as Box<dyn StatusBackend + Send>
        }
        _ => Box::new(TermcolorStatusBackend::new(ChatterLevel::Normal))
            as Box<dyn StatusBackend + Send>,
    };

    args.exec(status);
}

#[derive(Debug, Parser)]
struct ToplevelArgs {
    #[command(subcommand)]
    action: Action,
}

impl ToplevelArgs {
    fn exec(self, mut status: Box<dyn StatusBackend + Send>) {
        let result = match self.action {
            // Here we jump through hoops so that `build` can take ownership of
            // the status backend; it needs this to pass it around the async
            // framework.
            Action::Build(a) => {
                a.exec(status);
                return;
            }

            Action::FirstPassImpl(a) => a.exec(status.as_mut()),
            Action::SecondPassImpl(a) => a.exec(status.as_mut()),
            Action::Serve(a) => a.exec(status.as_mut()),
        };

        if let Err(e) = result {
            status.report_error(&e);
            std::process::exit(1)
        }
    }
}

#[derive(Debug, Subcommand)]
enum Action {
    Build(build::BuildArgs),
    FirstPassImpl(pass1::FirstPassImplArgs),
    SecondPassImpl(pass2::SecondPassImplArgs),
    Serve(serve::ServeArgs),
}
