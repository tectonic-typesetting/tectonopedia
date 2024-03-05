// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Creating the entrypoint HTML file that drives the Parcel.js
//! build process.

use sha2::Digest;
use std::{fs::File, io::Write};
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;

use crate::{
    cache::{Cache, OpCacheData},
    index::IndexCollection,
    operation::{DigestComputer, OpOutputStream, RuntimeEntityIdent},
};

/// Potentially emit the "entrypoint" files used to drive Parcel.js.
///
/// The return value is a list of identifiers of any entrypoints that were
/// modified during the build process.
pub fn maybe_make_entrypoint_operation(
    cache: &mut Cache,
    indices: &mut IndexCollection,
    status: &mut dyn StatusBackend,
) -> Result<Vec<RuntimeEntityIdent>> {
    let mut modified = Vec::new();

    // Set up the information about the operation. By construction, the
    // "outputs" index CSV file contains exactly what we need.

    let mut dc = DigestComputer::default();
    dc.update("make_entrypoint_v2");

    let input = RuntimeEntityIdent::new_other_file("cache/idx/outputs.csv", indices);
    input.update_digest(&mut dc, indices);

    let opid = dc.finalize();

    let needs_rerun = atry!(
        cache.operation_needs_rerun(&opid, indices, status);
        ["failed to probe cache for entrypoint creation operation"]
    );

    if !needs_rerun {
        return Ok(modified);
    }

    // It seems that we need to remake some part of the entrypoints.

    let mut ocd = OpCacheData::new(opid);
    ocd.add_input(input);

    // First, `_all.html`.
    //
    // NOTE: under current design, this is "other" not "output", because it
    // isn't an HTML file created during the TeX processing -- nothing in the
    // text should be able to reference it, after all.
    let output = RuntimeEntityIdent::new_other_file("build/_all.html", indices);
    let orig_digest = cache.unconditional_entity(output, indices)?.value_digest;

    let csv_path = indices.path_for_runtime_ident(input).unwrap();
    let csv_file = atry!(
        File::open(&csv_path);
        ["failed to open input `{}`", csv_path.display()]
    );

    let mut output_stream = atry!(
        OpOutputStream::new(output, indices);
        ["failed to open output file {:?}", output]
    );

    // By adding the reference to shared files at the top of the entrypoint, we
    // get Parcel.js to emit the associated built files under this file's name.
    // Otherwise they get tied to whatever happens to be the first entry that we
    // emit.
    atry!(
        writeln!(
            output_stream,
            "<link rel=\"stylesheet\" href=\"./tdux-fonts.css\">\n\
            <script src=\"../web/index.ts\" type=\"module\"></script>"
        );
        ["error writing to output {:?}", output]
    );

    let mut r = csv::Reader::from_reader(csv_file);

    for rec in r.records() {
        let rec = atry!(
            rec;
            ["error reading input `{}`", csv_path.display()]
        );

        atry!(
            writeln!(output_stream, "<a href=\"{}\"></a>", rec.get(0).unwrap());
            ["error writing to output {:?}", output]
        );
    }

    // ... wrap up `_all.html`.

    let (entity, size) = atry!(
        output_stream.close();
        ["failed to close output file {:?}", output]
    );

    ocd.add_output_with_value(output, entity.value_digest, size);

    if entity.value_digest != orig_digest {
        modified.push(output);
    }

    // Now, the `entrypoint.ts` file.
    //
    // The "indexUrl" construct gives us the URL of the search index data, which
    // we'll load on the fly if needed. It needs to be defined alongside the
    // JSON file because Parcel gives the JSON a magic hashed URL that we need
    // to propagate into the `web/` code. We have to give the file an extension
    // that isn't `.json` because otherwise Parcel will try to be smart and
    // inline the JSON data, breaking the scheme.

    let output = RuntimeEntityIdent::new_other_file("build/entrypoint.ts", indices);
    let orig_digest = cache.unconditional_entity(output, indices)?.value_digest;
    let mut output_stream = atry!(
        OpOutputStream::new(output, indices);
        ["failed to open output file {:?}", output]
    );

    atry!(
        writeln!(
            output_stream,
            r#"import {{ buildSpecificSettings }} from "../web/base.js";
buildSpecificSettings.indexUrl = require("url:./search_index.json.data");
import {{ mountIt }} from "../web/index.js";
mountIt(document);"#
        );
        ["error writing to output {:?}", output]
    );

    let (entity, size) = atry!(
        output_stream.close();
        ["failed to close output file {:?}", output]
    );

    ocd.add_output_with_value(output, entity.value_digest, size);

    if entity.value_digest != orig_digest {
        modified.push(output);
    }

    // All done.

    atry!(
        cache.finalize_operation(ocd, indices);
        ["failed to store caching information for entrypoint creation operation"]
    );

    Ok(modified)
}
