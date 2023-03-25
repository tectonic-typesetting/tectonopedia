// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Merging and emitting the "assets" used in the pass-2 Tectonic build.

use sha2::Digest;
use std::fs::File;
use tectonic::{
    config::PersistentConfig,
    driver::{OutputFormat, PassSetting, ProcessingSessionBuilder},
    errors::{Error as OldError, SyncError},
    status::termcolor::TermcolorStatusBackend,
    unstable_opts::UnstableOptions,
};
use tectonic_bridge_core::{SecuritySettings, SecurityStance};
use tectonic_engine_spx2html::AssetSpecification;
use tectonic_errors::prelude::*;
use tectonic_status_base::{ChatterLevel, StatusBackend};

use crate::{
    cache::{Cache, OpCacheData},
    index::IndexCollection,
    operation::{DigestComputer, OpOutputStream, RuntimeEntity, RuntimeEntityIdent},
};

pub fn maybe_asset_merge_operation(
    indices: &mut IndexCollection,
    asset_ids: &[RuntimeEntityIdent],
    cache: &mut Cache,
    status: &mut dyn StatusBackend,
) -> Result<RuntimeEntityIdent> {
    // Set up the information about the operation. The operation identifier
    // must include *all* inputs since if, say, we add a new one, we'll need
    // to rerun the op, and a simple check that all of the old inputs are
    // unchanged won't catch that.

    let mut dc = DigestComputer::default();
    dc.update("merge_assets_v1");

    for input in asset_ids {
        input.update_digest(&mut dc, indices);
    }

    let opid = dc.finalize();

    let output = RuntimeEntityIdent::new_other_file("cache/assets.json", indices);

    let needs_rerun = atry!(
        cache.operation_needs_rerun(&opid, indices, status);
        ["failed to probe cache for asset merge operation"]
    );

    if !needs_rerun {
        return Ok(output);
    }

    // It seems that we need to rerun the asset merge.
    //
    // Maybe it would help to try to set things up so that the merged result
    // doesn't change for unimportant changes such as ordering of inputs? Right
    // now we make no efforts in that direction.

    let mut ocd = OpCacheData::new(opid);
    let mut merged = AssetSpecification::default();

    for input in asset_ids {
        ocd.add_input(*input);

        let assets_path = indices.path_for_runtime_ident(*input).unwrap();

        let assets_file = atry!(
            File::open(&assets_path);
            ["failed to open input `{}`", assets_path.display()]
        );

        atry!(
            merged.add_from_saved(assets_file);
            ["failed to import assets data"]
        );
    }

    // Emit, cache, and we're done!

    let mut output_stream = atry!(
        OpOutputStream::new(output, indices);
        ["failed to open output file {:?}", output]
    );

    atry!(
        merged.save(&mut output_stream);
        ["failed to write assets to output file {:?}", output]
    );

    let (entity, size) = atry!(
        output_stream.close();
        ["failed to close output file {:?}", output]
    );

    ocd.add_output_with_value(output, entity.value_digest, size);

    atry!(
        cache.finalize_operation(ocd, indices);
        ["failed to store caching information for indexing operation"]
    );

    Ok(output)
}

/// Potentially emit the actual supporting assets for the HTML outputs.
///
/// The return value is a vector of runtime file entities that *might* have been
/// modified during the build process. The associated digest values are the
/// digests of the outputs from *before* the operation was run. The caller can
/// compare those digests to what they are *after* the build to search for
/// changes.
pub fn maybe_emit_assets_operation(
    asset_file: RuntimeEntityIdent,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    status: &mut dyn StatusBackend,
) -> Result<Vec<RuntimeEntity>> {
    // Set up the information about the operation.

    let mut dc = DigestComputer::default();
    dc.update("emit_assets_v1");
    asset_file.update_digest(&mut dc, indices);

    let opid = dc.finalize();

    let needs_rerun = atry!(
        cache.operation_needs_rerun(&opid, indices, status);
        ["failed to probe cache for asset merge operation"]
    );

    if !needs_rerun {
        // If we're not rerunning the operation, nothing should have changed!
        return Ok(Vec::new());
    }

    // It seems that we need to re-emit the assets.

    let mut ocd = OpCacheData::new(opid);
    ocd.add_input(asset_file);

    let assets_path = indices.path_for_runtime_ident(asset_file).unwrap();

    let assets_file = atry!(
        File::open(&assets_path);
        ["failed to open input `{}`", assets_path.display()]
    );

    let mut assets = AssetSpecification::default();

    atry!(
        assets.add_from_saved(assets_file);
        ["failed to import assets data"]
    );

    let mut outputs = Vec::new();

    for path in assets.output_paths() {
        // Register the outputs and compute their *pre-build* digests.
        let ident = RuntimeEntityIdent::new_output_file(path, indices);
        ocd.add_output(ident);
        outputs.push(cache.unconditional_entity(ident, indices)?);
    }

    atry!(
        emit_assets(assets, status).map_err(|e| SyncError::new(e));
        ["failed to emit Tectonic HTML assets"]
    );

    // Wrap up

    atry!(
        cache.finalize_operation(ocd, indices);
        ["failed to store caching information for asset emission operation"]
    );

    Ok(outputs)
}

fn emit_assets(assets: AssetSpecification, status: &mut dyn StatusBackend) -> Result<(), OldError> {
    // Suboptimal: this is basically copy-paste from the pass2 code.
    let config = PersistentConfig::open(false)?;
    let bundle = config.default_bundle(false, status)?;
    let format_cache_path = config.format_cache_path()?;
    let security = SecuritySettings::new(SecurityStance::MaybeAllowInsecures);
    let root = crate::config::get_root()?;

    let mut cls = root.clone();
    cls.push("cls");
    let unstables = UnstableOptions {
        extra_search_paths: vec![cls],
        ..UnstableOptions::default()
    };

    let mut out_dir = root.clone();
    out_dir.push("staging");
    std::fs::create_dir_all(&out_dir)?;

    let input = "\\newif\\ifpassone \
        \\passonetrue \
        \\input{preamble} \
        \\pediafinalemitfalse \
        \\input{postamble}\n";

    let mut sess = ProcessingSessionBuilder::new_with_security(security);
    sess.primary_input_buffer(&input.as_bytes())
        .tex_input_name("texput")
        .build_date(std::time::SystemTime::now())
        .bundle(bundle)
        .format_name("latex")
        .output_format(OutputFormat::Html)
        .html_precomputed_assets(assets)
        .filesystem_root(&root)
        .unstables(unstables)
        .format_cache_path(format_cache_path)
        .output_dir(&out_dir)
        .html_emit_assets(true)
        .pass(PassSetting::Default);

    let mut sess = sess.create(status)?;

    // Set up a quiet status backend to suppress the regular engine output here.
    let mut quiet_status = TermcolorStatusBackend::new(ChatterLevel::Minimal);
    sess.run(&mut quiet_status)?;

    Ok(())
}
