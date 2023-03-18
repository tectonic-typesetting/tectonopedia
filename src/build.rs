// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! The main TeX-to-HTML build operation.

use clap::Args;
use std::{fs, io::ErrorKind, time::Instant};
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_error, tt_note, StatusBackend};

use crate::{assets, cache, entrypoint_file, index, inputs, pass1, pass2, tex_pass};

fn build_implementation(status: &mut dyn StatusBackend) -> Result<()> {
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

    // Generate the merged asset info

    let merged_assets_id =
        assets::maybe_asset_merge_operation(&mut indices, &asset_ids[..], &mut cache, status)?;
    tt_note!(status, "refreshed merged asset description");

    // TeX pass 2, emitting

    let mut p2r = pass2::Pass2Processor::new(metadata_ids, merged_assets_id, &indices)?;
    tex_pass::process_inputs(&inputs, &mut p2r, &mut cache, &mut indices, status)?;
    let (n_outputs_rerun, n_outputs_total) = p2r.n_outputs();
    tt_note!(
            status,
            "refreshed TeX pass 2 outputs       - recreated {n_outputs_rerun} out of {n_outputs_total} HTML outputs"
        );

    // TODO: find a way to emit the HTML assets standalone!!!

    // Generate the entrypoint file

    entrypoint_file::maybe_make_entrypoint_operation(&mut cache, &mut indices, status)?;
    tt_note!(status, "refreshed entrypoint file");
    Ok(())
}

/// The standalone build operation.
#[derive(Args, Debug)]
pub struct BuildArgs {
    #[arg(long)]
    sample: Option<String>,
}

impl BuildArgs {
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        let t0 = Instant::now();

        // "Claim" the existing build tree to enable an incremental build,
        // or create a new one from scratch if needed.

        match fs::rename("build", "staging") {
            // Success - we will do a nice incremental build
            //
            // On Unix this operation will succeed if `staging` exists but is
            // empty. There is an unstable ErrorKind::DirectoryNotEmpty that
            // could indicate when it is *not* empty, which would suggest a
            // clash of builds.
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

        // Main build.

        build_implementation(status)?;

        tt_note!(
            status,
            "primary build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );

        // De-stage for `yarn` ops

        atry!(
            fs::rename("staging", "build");
            ["failed to rename `staging` to `build`"]
        );

        // TODO: yarn index, yarn build

        // All done.

        tt_note!(
            status,
            "full build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
        Ok(())
    }
}
