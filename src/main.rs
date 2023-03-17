// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

use clap::{Args, Parser, Subcommand};
use std::time::Instant;
use tectonic::status::termcolor::TermcolorStatusBackend;
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_note, ChatterLevel, StatusBackend};

mod assets;
mod cache;
mod config;
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

        // Set up data structures

        let mut indices = index::IndexCollection::new()?;

        atry!(
            indices.load_user_indices();
            ["failed to load user indices"]
        );

        let mut cache = atry!(
            cache::Cache::new(&mut indices, status);
            ["error initializing build cache"]
        );

        // Collect all of the inputs. With the way that we make the build
        // incremental, it makes the most sense to just put them all in a big vec.

        let inputs = atry!(
            inputs::collect_inputs(&mut indices);
            ["failed to scan list of input files"]
        );

        // First TeX pass of indexing and gathering font/asset information.

        let mut p1r = pass1::Pass1Processor::new(&indices);
        let ninputs =
            tex_pass::process_inputs(&inputs, &mut p1r, &mut cache, &mut indices, status)?;
        tt_note!(status, "TeX pass 1: complete - processed {ninputs} inputs");
        let (asset_ids, metadata_ids) = p1r.unpack();

        // Resolve cross-references and validate.

        index::maybe_indexing_operation(&mut indices, &metadata_ids[..], &mut cache, status)?;

        // Generate the merged asset info

        assets::maybe_asset_merge_operation(&mut indices, &asset_ids[..], &mut cache, status)?;

        // atry!(
        //     indices.validate_references();
        //     ["failed to validate cross-references"]
        // );
        // tt_note!(
        //     status,
        //     "index validation: complete - {}",
        //     indices.index_summary()
        // );
        //
        // // TeX pass 2, emitting
        //
        // let mut entrypoints_file = atry!(
        //     File::create("build/_all.html");
        //     ["error creating output `build/_all.html`"]
        // );
        //
        // // By adding the reference to shared files here at the top of this
        // // entrypoint, we get Parcel.js to emit the associated built files under
        // // this file's name. Otherwise they get tied to whatever happens to be
        // // the first entry that we emit.
        // atry!(
        //     writeln!(
        //         entrypoints_file,
        //         "<link rel=\"stylesheet\" href=\"./tdux-fonts.css\">\n\
        //         <script src=\"../web/index.ts\" type=\"module\"></script>"
        //     );
        //     ["error writing to output `build/_all.html`"]
        // );
        //
        // let mut p2r = pass2::Pass2Processor::new(assets, indices, entrypoints_file);
        // tex_pass::process_inputs(&mut p2r, status)?;
        // let n_outputs = p2r.n_outputs();
        // tt_note!(status, "TeX pass 2: complete - created {n_outputs} outputs");
        //
        tt_note!(
            status,
            "build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
        Ok(())
    }
}
