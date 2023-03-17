// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Merging and emitting the "assets" used in the pass-2 Tectonic build.

use sha2::Digest;
use std::fs::File;
use tectonic_engine_spx2html::AssetSpecification;
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;

use crate::{
    cache::{Cache, OpCacheData},
    index::IndexCollection,
    operation::{DigestComputer, OpOutputStream, RuntimeEntityIdent},
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
        eprintln!("assets short-circuit");
        return Ok(output);
    }

    // It seems that we need to rerun the asset merge.

    eprintln!("assets running it");
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
