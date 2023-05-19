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
    io::{ErrorKind, Write},
    path::Path,
    time::{Duration, Instant},
};
use tectonic::status::termcolor::TermcolorStatusBackend;
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_error, tt_note, ChatterLevel, StatusBackend};

use crate::{assets, cache, entrypoint_file, index, inputs, pass1, pass2, tex_pass, yarn};

/// The return value is potentially a list of the final outputs that were
/// modified during this build process, if the boolean argument is true. The
/// list may be empty if nothing actually changed, or if the argument is false.
/// This list is used in "watch" mode to efficiently update Parcel.js.
fn primary_build_implementation(
    collect_paths: bool,
    terse_output: bool,
    status: &mut dyn StatusBackend,
) -> Result<Vec<String>> {
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

    if terse_output {
        print!("pass1");
        let _ignored = std::io::stdout().flush();
    } else {
        tt_note!(
            status,
            "refreshed TeX pass 1 outputs       - processed {n_processed} of {} inputs",
            inputs.len()
        );
    }

    let (asset_ids, metadata_ids) = p1r.unpack();

    // Resolve cross-references and validate.

    index::construct_indices(&mut indices, &metadata_ids[..], &mut cache, status)?;

    if terse_output {
        print!(" cross-index");
        let _ignored = std::io::stdout().flush();
    } else {
        tt_note!(
            status,
            "refreshed cross indices            - {}",
            indices.index_summary()
        );
    }

    // Generate the merged asset info and emit the files. Start collecting
    // information about our outputs that will feed into the Parcel.js build
    // process, specifically which ones have actually been modified. We use that
    // for efficient updates in the "watch" mode.

    let merged_assets_id =
        assets::maybe_asset_merge_operation(&mut indices, &asset_ids[..], &mut cache, status)?;

    if !terse_output {
        tt_note!(status, "refreshed merged asset description");
    }

    let mut maybe_modified_output_files =
        assets::maybe_emit_assets_operation(merged_assets_id, &mut cache, &mut indices, status)?;

    if terse_output {
        print!(" assets");
        let _ignored = std::io::stdout().flush();
    } else {
        tt_note!(status, "refreshed HTML support assets");
    }

    // TeX pass 2, emitting

    let mut p2r = pass2::Pass2Processor::new(metadata_ids, merged_assets_id, &indices)?;
    tex_pass::process_inputs(&inputs, &mut p2r, &mut cache, &mut indices, status)?;
    let (n_outputs_rerun, n_outputs_total) = p2r.n_outputs();

    if terse_output {
        print!(" pass2");
        let _ignored = std::io::stdout().flush();
    } else {
        tt_note!(
            status,
            "refreshed TeX pass 2 outputs       - recreated {n_outputs_rerun} out of {n_outputs_total} HTML outputs"
        );
    }

    maybe_modified_output_files.append(&mut p2r.into_potential_modified_outputs());

    // Generate the entrypoint file, and start generating the list of output
    // files that actually *were* modified. Unlike the TeX pass 2 and assets
    // steps, it's convenient for the entrypoint stage to figure out whether the
    // output actually changed or not.

    let mut modified_output_files = Vec::new();

    let id = entrypoint_file::maybe_make_entrypoint_operation(&mut cache, &mut indices, status)?;

    if terse_output {
        print!(" entrypoint");
        let _ignored = std::io::stdout().flush();
    } else {
        tt_note!(status, "refreshed entrypoint file");
    }

    if let Some(id) = id {
        modified_output_files.push(id);
    }

    // Figure out which of the other outputs have been modified.

    for output in maybe_modified_output_files.drain(..) {
        let updated = cache.require_entity(output.ident, &indices)?;

        if updated.value_digest != output.value_digest {
            modified_output_files.push(output.ident);
        }
    }

    // TODO: rewrite the cache file state info!!!

    // Translate the entity IDs into relative paths, if we care. That conversion
    // relies on the IndexCollection, which we're about to throw away, which is
    // why we leave the "ident" space

    let paths = if collect_paths {
        modified_output_files
            .into_iter()
            .filter_map(|o| indices.relpath_for_output_file(o))
            .map(|o| o.to_owned())
            .collect()
    } else {
        Vec::new()
    };

    Ok(paths)
}

fn build_through_index(
    do_rename: bool,
    collect_paths: bool,
    terse_output: bool,
    status: &mut dyn StatusBackend,
) -> Result<(Instant, Vec<String>)> {
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

                Err(e) => {
                    return Err(e).context("failed to create directory `staging`".to_string())
                }
            },

            // Some other problem - bail.
            Err(e) => return Err(e).context("failed to rename `build` to `staging`".to_string()),
        }
    }

    // Main build.

    let modified_files = primary_build_implementation(collect_paths, terse_output, status)?;

    if !terse_output {
        tt_note!(
            status,
            "primary build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
    }

    // De-stage for `yarn` ops and make the fulltext index.

    atry!(
        fs::rename("staging", "build");
        ["failed to rename `staging` to `build`"]
    );

    atry!(
        yarn::yarn_index(terse_output, status);
        ["failed to generate fulltext index"]
    );

    if terse_output {
        print!(" index-text");
        let _ignored = std::io::stdout().flush();
    }

    Ok((t0, modified_files))
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
        let (t0, _) = build_through_index(true, false, false, status)?;

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
        let mut yarn_child = yarn::yarn_serve()?;

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

        let _ignore = yarn_child.wait();
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

        println!();
        let (t0, changed) = build_through_index(self.last_succeeded, true, true, status)?;
        println!(" ... {:.1} seconds elapsed\n\n", t0.elapsed().as_secs_f32());

        // Touch the modified files; Parcel's change-detection code doesn't
        // catch atomic replacements.

        for output in &changed {
            let p = format!("build{}{}", std::path::MAIN_SEPARATOR, output);

            atry!(
                filetime::set_file_mtime(&p, filetime::FileTime::now());
                ["unable to update modification time of file `{}`", p]
            );
        }

        Ok(())
    }
}
