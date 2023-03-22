// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! The main TeX-to-HTML build operation.
//!
//! The most interesting piece here is the `watch` operation. It's a bit tricky
//! since we need to cooperate with Parcel.js's `serve` operation. In
//! particular, we want just one `yarn serve` hanging out doing its thing, but
//! we don't want it rebuilding with a bunch of partial inputs as we do the TeX
//! processing.
//!
//! Based on my Linux testing, it looks like we can move the `build` directory
//! out from under `yarn serve` and then back, and Parcel.js won't trigger a
//! rebuild even if any files in that tree have changed. But if we then make any
//! updates in that tree, Parcel will detect a change and rebuild everything. So
//! our strategy is to move `build` to a temporary name (`staging`) for the TeX
//! phase, move it back to `build` when that's all done, and then do the `yarn
//! index` step to trigger the Parcel rebuild (as well as because we need to do
//! it and it can only run when all of the TeX stuff is done).

use clap::Args;
use notify_debouncer_mini::{new_debouncer, notify, DebounceEventHandler, DebounceEventResult};
use std::{
    fs,
    io::ErrorKind,
    path::Path,
    time::{Duration, Instant},
};
use tectonic::status::termcolor::TermcolorStatusBackend;
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_error, tt_note, ChatterLevel, StatusBackend};

use crate::{assets, cache, entrypoint_file, index, inputs, pass1, pass2, tex_pass, yarn};

fn primary_build_implementation(status: &mut dyn StatusBackend) -> Result<()> {
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

    let mut p1r = pass1::Pass1Processor::default();
    let n_processed =
        tex_pass::process_inputs(&inputs, &mut p1r, &mut cache, &mut indices, status)?;
    tt_note!(
        status,
        "refreshed TeX pass 1 outputs       - processed {n_processed} of {} inputs",
        inputs.len()
    );
    let (asset_ids, metadata_ids) = p1r.unpack();

    // Resolve cross-references and validate.

    index::construct_indices(&mut indices, &metadata_ids[..], &mut cache, status)?;
    tt_note!(
        status,
        "refreshed internal indices         - {}",
        indices.index_summary()
    );

    // Generate the merged asset info and emit the files

    let merged_assets_id =
        assets::maybe_asset_merge_operation(&mut indices, &asset_ids[..], &mut cache, status)?;
    tt_note!(status, "refreshed merged asset description");

    assets::maybe_emit_assets_operation(merged_assets_id, &mut cache, &mut indices, status)?;
    tt_note!(status, "refreshed HTML support assets");

    // TeX pass 2, emitting

    let mut p2r = pass2::Pass2Processor::new(metadata_ids, merged_assets_id, &indices)?;
    tex_pass::process_inputs(&inputs, &mut p2r, &mut cache, &mut indices, status)?;
    let (n_outputs_rerun, n_outputs_total) = p2r.n_outputs();
    tt_note!(
            status,
            "refreshed TeX pass 2 outputs       - recreated {n_outputs_rerun} out of {n_outputs_total} HTML outputs"
        );

    // Generate the entrypoint file

    entrypoint_file::maybe_make_entrypoint_operation(&mut cache, &mut indices, status)?;
    tt_note!(status, "refreshed entrypoint file");
    Ok(())
}

fn build_through_index(do_rename: bool, status: &mut dyn StatusBackend) -> Result<Instant> {
    let t0 = Instant::now();

    // "Claim" the existing build tree to enable an incremental build,
    // or create a new one from scratch if needed.

    if do_rename {
        match fs::rename("build", "staging") {
            // Success - we will do a nice incremental build
            //
            // On Unix this operation will succeed if `staging` exists but is
            // empty; the empty directory will be replaced. There is an unstable
            // ErrorKind::DirectoryNotEmpty that could indicate when it is *not*
            // empty, which would suggest a clash of builds.
            Ok(_) => {}

            // No existing build; we'll start from scratch.
            Err(ref e) if e.kind() == ErrorKind::NotFound => match fs::create_dir("staging") {
                Ok(_) => {}

                Err(ref e) if e.kind() == ErrorKind::AlreadyExists => {
                    tt_error!(
                        status,
                        "failed to create directory `staging` - it already exists"
                    );
                    tt_error!(status, "is another \"build\" or \"watch\" process running?");
                    // TODO: add a --force option and/or a `clean` subcommand
                    tt_error!(status, "if not, remove the directory and try again");
                    bail!("cannot proceed with build");
                }

                Err(e) => return Err(e).context(format!("failed to create directory `staging`")),
            },

            // Some other problem - bail.
            Err(e) => return Err(e).context(format!("failed to rename `build` to `staging`")),
        }
    }

    // Main build.

    primary_build_implementation(status)?;

    tt_note!(
        status,
        "primary build took {:.1} seconds",
        t0.elapsed().as_secs_f32()
    );

    // De-stage for `yarn` ops and make the fulltext index.

    atry!(
        fs::rename("staging", "build");
        ["failed to rename `staging` to `build`"]
    );

    atry!(
        yarn::yarn_index(status);
        ["failed to generate fulltext index"]
    );

    Ok(t0)
}

/// The standalone build operation.
#[derive(Args, Debug)]
pub struct BuildArgs {
    #[arg(long)]
    sample: Option<String>,
}

impl BuildArgs {
    /// In the "build" op, we do the main build, then just cap it off with a
    /// `yarn build` and we're done.
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        let t0 = build_through_index(true, status)?;

        atry!(
            yarn::yarn_build(status);
            ["failed to generate production files"]
        );

        tt_note!(
            status,
            "full build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
        Ok(())
    }
}

/// The watch operation.
#[derive(Args, Debug)]
pub struct WatchArgs {}

impl WatchArgs {
    /// This function is special since it takes ownership of the status backend,
    /// since we need to send it to another thread for steady-state operations.
    pub fn exec(self, status: Box<dyn StatusBackend + Send>) {
        // Set up our object that will watch for changes, and run an initial
        // build.

        let mut watcher = Watcher {
            status,
            last_succeeded: true,
        };

        watcher.build();

        // OK well now let's create another status backend that we'll use in
        // case anything goes wrong from here on out. I think it's better to
        // send the "original" status to the watcher since it will be set up
        // with whatever configuration and customization happens by default.

        let mut status = TermcolorStatusBackend::new(ChatterLevel::Normal);

        if let Err(e) = self.finish_exec(watcher) {
            status.report_error(&e);
            std::process::exit(1)
        }
    }

    fn finish_exec(self, watcher: Watcher) -> Result<()> {
        eprintln!("XXX launch yarn start!");

        // Now set up the debounced notifier that will handle things from here
        // on out.

        let mut debouncer = atry!(
            new_debouncer(Duration::from_millis(300), None, watcher);
            ["failed to set up filesystem change notifier"]
        );

        for dname in &["cls", "idx", "src", "txt", "web"] {
            atry!(
                debouncer
                    .watcher()
                    .watch(Path::new(dname), notify::RecursiveMode::Recursive);
                ["failed to watch directory `{}`", dname]
            );
        }

        // XXX FAKE
        std::thread::sleep(Duration::from_secs(99999));
        Ok(())
    }
}

struct Watcher {
    status: Box<dyn StatusBackend + Send>,
    last_succeeded: bool,
}

impl DebounceEventHandler for Watcher {
    fn handle_event(&mut self, event: DebounceEventResult) {
        if let Err(mut e) = event {
            tt_error!(
                self.status.as_mut(),
                "file change notification system returned error(s)"
            );

            for e in e.drain(..) {
                tt_error!(self.status.as_mut(), "failed to detect changes"; e.into());
            }
        } else {
            self.build();
        }
    }
}

impl Watcher {
    fn build(&mut self) {
        if let Err(e) = self.build_inner() {
            self.status.report_error(&e);
            self.last_succeeded = false;
        } else {
            self.last_succeeded = true;
        }
    }

    fn build_inner(&mut self) -> Result<()> {
        let status = self.status.as_mut();

        let t0 = build_through_index(self.last_succeeded, status)?;

        tt_note!(
            status,
            "primary build plus index took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
        Ok(())
    }
}
