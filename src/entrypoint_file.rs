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

pub fn maybe_make_entrypoint_operation(
    cache: &mut Cache,
    indices: &mut IndexCollection,
    status: &mut dyn StatusBackend,
) -> Result<RuntimeEntityIdent> {
    // Set up the information about the operation. By construction, the
    // "outputs" index CSV file contains exactly what we need.

    let mut dc = DigestComputer::default();
    dc.update("make_entrypoint_v1");

    let input = RuntimeEntityIdent::new_other_file("cache/idx/outputs.csv", indices);
    input.update_digest(&mut dc, indices);

    let opid = dc.finalize();

    // NOTE: under current design, this is "other" not "output"; maybe that will change?
    let output = RuntimeEntityIdent::new_other_file("build/_all.html", indices);

    let needs_rerun = atry!(
        cache.operation_needs_rerun(&opid, indices, status);
        ["failed to probe cache for entrypoint creation operation"]
    );

    if !needs_rerun {
        return Ok(output);
    }

    // It seems that we need to remake the entrypoint. Set up the files ...

    let mut ocd = OpCacheData::new(opid);
    ocd.add_input(input);

    let csv_path = indices.path_for_runtime_ident(input).unwrap();

    let csv_file = atry!(
        File::open(&csv_path);
        ["failed to open input `{}`", csv_path.display()]
    );

    let mut output_stream = atry!(
        OpOutputStream::new(output, indices);
        ["failed to open output file {:?}", output]
    );

    // ... do the transform ...
    //
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

    // ... wrap up, and we're done!

    let (entity, size) = atry!(
        output_stream.close();
        ["failed to close output file {:?}", output]
    );

    ocd.add_output_with_value(output, entity.value_digest, size);

    atry!(
        cache.finalize_operation(ocd, indices);
        ["failed to store caching information for entrypoint creation operation"]
    );

    Ok(output)
}
