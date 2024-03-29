// Copyright 2022-2024 the Tectonic Project
// Licensed under the MIT License

//! The main TeX-to-HTML build operation.

use clap::Args;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;
use tokio::task::spawn_blocking;

use crate::{
    assets, cache, entrypoint_file, index, inputs,
    messages::{
        new_sync_bus_channel, BuildCompleteMessage, CliStatusMessageBus, Message, MessageBus,
    },
    operation::{RuntimeEntity, RuntimeEntityIdent},
    pass1, pass2, tex_pass, yarn,
};

/// The return value is potentially a list of the final outputs that were
/// modified during this build process, if the boolean argument is true. The
/// list may be empty if nothing actually changed, or if the argument is false.
/// This list is used in "serve" mode to efficiently update Parcel.js.
async fn primary_build_implementation<T: MessageBus + 'static>(
    n_workers: usize,
    collect_paths: bool,
    mut bus: T,
) -> Result<Vec<String>> {
    // Set up data structures. Here the return type of spawn_blocking is a
    // Result<Result<IndexCollection>, JoinError>, so we have to double-unwrap
    // it.

    let (mut bus_tx, bus_rx) = new_sync_bus_channel();

    let handle = spawn_blocking(move || -> Result<(index::IndexCollection, cache::Cache, Vec<RuntimeEntityIdent>)> {
        bus_tx.post(Message::PhaseStarted("load-indices".into()));

        let mut indices = index::IndexCollection::new()?;
        atry!(
            indices.load_user_indices();
            ["failed to load user indices"]
        );

        bus_tx.post(Message::PhaseStarted("load-cache".into()));

        let cache = atry!(
            cache::Cache::new(&mut indices, &mut bus_tx);
            ["error initializing build cache"]
        );

        bus_tx.post(Message::PhaseStarted("collect-inputs".into()));

        // Collect all of the inputs. With the way that we make the build
        // incremental, it makes the most sense to just put them all in a big vec.

        let inputs = atry!(
            inputs::collect_inputs(&mut indices);
            ["failed to scan list of input files"]
        );

        Ok((indices, cache, inputs))
    });

    bus_rx.drain(bus.clone()).await;
    let (mut indices, mut cache, inputs) = handle.await??;

    // First TeX pass of indexing and gathering font/asset information.

    bus.post(Message::PhaseStarted("pass-1".into())).await;

    let mut p1r = pass1::Pass1Processor::default();
    let _n_processed = tex_pass::process_inputs(
        &inputs,
        n_workers,
        &mut p1r,
        &mut cache,
        &mut indices,
        bus.clone(),
    )
    .await?;

    let (mut bus_tx, bus_rx) = new_sync_bus_channel();

    let handle = spawn_blocking(
        #[allow(clippy::type_complexity)]
        move || -> Result<(Vec<RuntimeEntityIdent>, RuntimeEntityIdent, Vec<RuntimeEntity>, index::IndexCollection, cache::Cache)> {
            let (asset_ids, metadata_ids) = p1r.unpack();

            // Resolve cross-references and validate.

            index::construct_indices(&mut indices, &metadata_ids[..], &mut cache, &mut bus_tx)?;

            // Generate the merged asset info and emit the files. Start collecting
            // information about our outputs that will feed into the Parcel.js build
            // process, specifically which ones have actually been modified. We use that
            // for efficient updates in the "serve" mode.

            let merged_assets_id = assets::maybe_asset_merge_operation(
                &mut indices,
                &asset_ids[..],
                &mut cache,
                &mut bus_tx,
            )?;

            let maybe_modified_output_files = assets::maybe_emit_assets_operation(
                merged_assets_id,
                &mut cache,
                &mut indices,
                &mut bus_tx,
            )?;

            Ok((metadata_ids, merged_assets_id, maybe_modified_output_files, indices, cache))
        },
    );

    bus_rx.drain(bus.clone()).await;
    let (metadata_ids, merged_assets_id, mut maybe_modified_output_files, mut indices, mut cache) =
        handle.await??;

    // TeX pass 2, emitting

    bus.post(Message::PhaseStarted("pass-2".into())).await;

    let mut p2r = pass2::Pass2Processor::new(metadata_ids, merged_assets_id, &indices)?;
    tex_pass::process_inputs(
        &inputs,
        n_workers,
        &mut p2r,
        &mut cache,
        &mut indices,
        bus.clone(),
    )
    .await?;
    let (_n_outputs_rerun, _n_outputs_total) = p2r.n_outputs();

    maybe_modified_output_files.append(&mut p2r.into_potential_modified_outputs());

    let (mut bus_tx, bus_rx) = new_sync_bus_channel();

    let handle = spawn_blocking(
        move || -> Result<(Vec<RuntimeEntityIdent>, index::IndexCollection)> {
            // Generate the entrypoint file, and start generating the list of output
            // files that actually *were* modified. Unlike the TeX pass 2 and assets
            // steps, it's convenient for the entrypoint stage to figure out whether the
            // output actually changed or not.

            let mut modified_output_files = entrypoint_file::maybe_make_entrypoint_operation(
                &mut cache,
                &mut indices,
                &mut bus_tx,
            )?;

            // Figure out which of the other outputs have been modified.

            for output in maybe_modified_output_files.drain(..) {
                let updated = cache.require_entity(output.ident, &indices)?;

                if updated.value_digest != output.value_digest {
                    modified_output_files.push(output.ident);
                }
            }

            Ok((modified_output_files, indices))
        },
    );

    bus_rx.drain(bus.clone()).await;
    let (modified_output_files, indices) = handle.await??;

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

/// The returned value is a list of the output files that were modified during
/// the build. The paths are relative to the `build/` directory.
pub async fn build_through_index<T: MessageBus + 'static>(
    n_workers: usize,
    collect_paths: bool,
    mut bus: T,
) -> Result<Vec<String>> {
    let result = primary_build_implementation(n_workers, collect_paths, bus.clone()).await;
    let modified_files = result?;

    bus.post(Message::PhaseStarted("index-text".into())).await;

    atry!(
        yarn::yarn_index(bus, true).await;
        ["failed to generate fulltext index"]
    );

    Ok(modified_files)
}

pub async fn debug_build_one_input<T: MessageBus + 'static>(
    input_path: &str,
    bus: T,
) -> Result<()> {
    let (mut bus_tx, bus_rx) = new_sync_bus_channel();

    let handle = spawn_blocking(move || -> Result<(index::IndexCollection, cache::Cache)> {
        let mut indices = index::IndexCollection::new()?;
        atry!(
            indices.load_user_indices();
            ["failed to load user indices"]
        );

        let cache = atry!(
            cache::Cache::new(&mut indices, &mut bus_tx);
            ["error initializing build cache"]
        );

        Ok((indices, cache))
    });

    bus_rx.drain(bus.clone()).await;
    let (mut indices, mut cache) = handle.await??;

    let input = indices.make_tex_source_ident(input_path);

    let mut p1r = pass1::Pass1Processor::default();

    tex_pass::debug_one_input(input, &mut p1r, &mut cache, &mut indices, bus).await
}

/// The standalone build operation.
#[derive(Args, Debug)]
pub struct BuildArgs {
    #[arg(long, short = 'j', default_value_t = 0)]
    parallel: usize,

    #[arg(long)]
    no_dist: bool,
}

impl BuildArgs {
    /// In the "build" op, we do the main build, then just cap it off with a
    /// `yarn build` and we're done.
    pub fn exec(self, status: Box<dyn StatusBackend + Send>) {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(self.inner(status));
    }

    async fn inner(self, status: Box<dyn StatusBackend + Send>) {
        let status = Arc::new(Mutex::new(status));
        let bus = CliStatusMessageBus::new_scaffold(status.clone());
        let result = self.double_inner(bus).await;
        let status = &mut **status.lock().unwrap();

        if let Err(e) = result {
            status.report_error(&e);
            std::process::exit(1)
        }
    }

    async fn double_inner(self, mut bus: CliStatusMessageBus) -> Result<()> {
        let n_workers = if self.parallel > 0 {
            self.parallel
        } else {
            num_cpus::get()
        };

        let t0 = Instant::now();
        build_through_index(n_workers, false, bus.clone()).await?;

        if !self.no_dist {
            bus.post(Message::PhaseStarted("yarn-build".into())).await;

            atry!(
                yarn::yarn_build(bus.clone(), false).await;
                ["failed to generate production files"]
            );
        }

        // Unlike the build-started message, which wouldn't cause our status
        // reporter to print anything, this message will.
        bus.post(Message::BuildComplete(BuildCompleteMessage {
            file: None,
            success: true,
            elapsed: t0.elapsed().as_secs_f32(),
        }))
        .await;

        Ok(())
    }
}
