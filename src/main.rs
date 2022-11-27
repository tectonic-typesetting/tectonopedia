// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use clap::{Args, Parser, Subcommand};
use std::{fs::File, io::Write, time::Instant};
use tectonic::status::termcolor::TermcolorStatusBackend;
use tectonic_engine_spx2html::AssetSpecification;
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_note, ChatterLevel, StatusBackend};

mod config;
mod inputs;
mod pass1;
mod pass2;
#[macro_use]
mod texworker;
mod worker_status;

use worker_status::WorkerStatusBackend;

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
    Build(BuildArgs),
    FirstPassImpl(pass1::FirstPassImplArgs),
    SecondPassImpl(pass2::SecondPassImplArgs),
}

#[derive(Args, Debug)]
struct BuildArgs {
    #[arg(long)]
    sample: Option<String>,
}

impl BuildArgs {
    fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        let t0 = Instant::now();

        // First pass of indexing and gathering font/asset information.

        let mut assets = AssetSpecification::default();

        let ninputs = texworker::process_inputs(
            pass1::Pass1Driver::default,
            |asset_text| {
                assets
                    .add_from_saved(asset_text.as_bytes())
                    .expect("failed to transfer assets JSON");
            },
            status,
        )?;

        tt_note!(status, "pass 1: complete - processed {} inputs", ninputs);

        // Indexing goes here!

        let mut entrypoints_file = atry!(
            File::create("build/_all.html");
            ["error creating output `build/_all.html`"]
        );

        texworker::process_inputs(
            || pass2::Pass2Driver::new(assets.clone()),
            |info| {
                for ep in info.entrypoints {
                    let _r = writeln!(entrypoints_file, "<a href=\"{}\"></a>", ep);
                }
            },
            status,
        )?;
        tt_note!(status, "pass 2: complete");

        tt_note!(
            status,
            "build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
        Ok(())
    }
}
