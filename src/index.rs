// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use std::{fs::File, io::Read};
use string_interner::{StringInterner, Symbol};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_error, StatusBackend};

use crate::{holey_vec::HoleyVec, multivec::MultiVec, worker_status::WorkerStatusBackend};

use string_interner::DefaultSymbol as EntryId;
pub type IndexId = EntryId;

#[derive(Debug)]
struct Index {
    entries: StringInterner,
    defs: Vec<Option<OutputLocation>>,
}

impl Index {
    fn new(_id: IndexId) -> Self {
        Index {
            entries: Default::default(),
            defs: Default::default(),
        }
    }

    /// Ensure that the specified name exists in the index.
    fn reference(&mut self, name: impl AsRef<str>) -> EntryId {
        self.entries.get_or_intern(name)
    }

    /// Ensure that the name exists in the index, and declare its point of
    /// definition as an IndexEntry.
    ///
    /// The operation can fail if the name has already had its location defined,
    /// and this definition is for a different location. In that case, the error
    /// value is the location of the previous definition.
    fn define(
        &mut self,
        name: impl AsRef<str>,
        loc: OutputLocation,
    ) -> Result<EntryId, OutputLocation> {
        let entry = self.reference(name);
        let eidx = entry.to_usize();

        // The Err case will always be Some because no error is returned if the
        // existing value is the default.
        if let Err(Some(prev_loc)) = self.defs.ensure_holey_slot_available(eidx) {
            if *prev_loc == loc {
                return Err(*prev_loc);
            }
        }

        self.defs[eidx] = Some(loc);
        Ok(entry)
    }

    #[inline(always)]
    fn get(&self, name: impl AsRef<str>) -> Option<EntryId> {
        self.entries.get(name)
    }

    /// Resolution should never fail unless there's an implementation bug that
    /// mixes up EntryIds for one index and another. So, to not have to drag
    /// Results around in a bunch of APIs that won't be actionable, we go ahead
    /// and unwrap the return value here.
    #[inline(always)]
    fn resolve(&self, entry: EntryId) -> &str {
        self.entries.resolve(entry).unwrap()
    }

    /// Return whether the specified entry has a definition location.
    fn has_definition(&self, entry: EntryId) -> bool {
        self.defs.holey_slot_is_filled(entry.to_usize())
    }

    fn iter(&self) -> impl IntoIterator<Item = (EntryId, &str)> {
        self.entries.into_iter()
    }
}

#[derive(Debug)]
pub struct IndexCollection {
    indices: Vec<Index>,

    /// Information about the index references contained in each input (which
    /// may spread over multiple outputs). We group by input because we need to
    /// feed the resolved references into each input for its second-pass
    /// processing. The multi-vec's "keys" are the to_usize() of the input
    /// EntryIds.
    refs: MultiVec<IndexEntry>,
}

pub const INDEX_OF_INDICES_NAME: &'static str = "ioi";
const INDEX_OF_INDICES_INDEX: usize = 0;

pub const INPUTS_INDEX_NAME: &'static str = "inputs";
const INPUTS_INDEX_INDEX: usize = 1;

pub const OUTPUTS_INDEX_NAME: &'static str = "outputs";
const OUTPUTS_INDEX_INDEX: usize = 2;

pub const FRAGMENTS_INDEX_NAME: &'static str = "fragments";
const FRAGMENTS_INDEX_INDEX: usize = 3;

impl IndexCollection {
    /// Define a new index by name.
    ///
    /// Indices cannot be redundantly declared.
    pub fn declare_index(&mut self, name: impl AsRef<str>) -> Result<IndexId> {
        let name = name.as_ref();
        let id: IndexId = self.indices[INDEX_OF_INDICES_INDEX].reference(name);
        let idx = id.to_usize();

        if idx != self.indices.len() {
            bail!("re-declaration of index `{}`", name);
        }

        self.indices.push(Index::new(id));
        Ok(id)
    }

    /// Convert an index name into its IndexId. The conversion can fail if the
    /// index in question was never declared.
    pub fn get_index(&self, name: impl AsRef<str>) -> Result<IndexId> {
        let name = name.as_ref();
        Ok(a_ok_or!(
            self.indices[INDEX_OF_INDICES_INDEX].get(name);
            ["no such index `{}`", name]
        ))
    }

    /// Create an OutputLocation.
    pub fn make_location_by_id(
        &mut self,
        output_id: IndexId,
        fragment: impl AsRef<str>,
    ) -> OutputLocation {
        OutputLocation::new(
            output_id,
            self.indices[FRAGMENTS_INDEX_INDEX].reference(fragment),
        )
    }

    pub fn reference_by_id(&mut self, index: IndexId, entry: impl AsRef<str>) -> EntryId {
        self.indices[index.to_usize()].reference(entry)
    }

    pub fn reference(&mut self, index: impl AsRef<str>, entry: impl AsRef<str>) -> Result<EntryId> {
        let id = self.get_index(index)?;
        Ok(self.indices[id.to_usize()].reference(entry))
    }

    pub fn reference_to_entry(
        &mut self,
        index: impl AsRef<str>,
        entry: impl AsRef<str>,
    ) -> Result<IndexEntry> {
        let index = self.get_index(index)?;
        let entry = self.indices[index.to_usize()].reference(entry);
        Ok(IndexEntry { index, entry })
    }

    pub fn define_by_id(
        &mut self,
        index: IndexId,
        entry: impl AsRef<str>,
        loc: OutputLocation,
    ) -> Result<EntryId> {
        let entry = entry.as_ref();

        self.indices[index.to_usize()].define(entry, loc).map_err(|prev_loc|
            anyhow!(
                "redefinition of entry `{}` in index `{}`; previous location was `{}{}`, new location is `{}{}`",
                entry,
                self.indices[INDEX_OF_INDICES_INDEX].resolve(index),
                self.indices[OUTPUTS_INDEX_INDEX].resolve(prev_loc.output),
                self.indices[FRAGMENTS_INDEX_INDEX].resolve(prev_loc.fragment),
                self.indices[OUTPUTS_INDEX_INDEX].resolve(loc.output),
                self.indices[FRAGMENTS_INDEX_INDEX].resolve(loc.fragment),
            )
        )
    }

    pub fn define(
        &mut self,
        index: impl AsRef<str>,
        entry: impl AsRef<str>,
        loc: OutputLocation,
    ) -> Result<EntryId> {
        let id = self.get_index(index)?;
        self.define_by_id(id, entry, loc)
    }

    pub fn resolve_by_id(&self, index: IndexId, entry: EntryId) -> &str {
        self.indices[index.to_usize()].resolve(entry)
    }

    /// Capture the set of index entries referenced by a particular input.
    ///
    /// The input is identified by its entry in the inputs index. Calling this
    /// function more than once for the same input ID is illegal, and will
    /// result in an error being returned.
    pub fn log_references(
        &mut self,
        input: EntryId,
        refs: impl IntoIterator<Item = IndexEntry>,
    ) -> Result<()> {
        self.refs.add_extend(input.to_usize(), refs)
    }

    /// Validate all of the cross-references.
    pub fn validate_references(&self) -> Result<()> {
        let mut n_failures = 0;

        for (input_id, input_name) in self.indices[INPUTS_INDEX_INDEX].iter() {
            let mut status = WorkerStatusBackend::new(input_name);

            // We always define the refs for every input, so this lookup can
            // never fail.
            let refs = self.refs.lookup(input_id.to_usize()).unwrap();

            for entry in refs {
                if !self.indices[entry.index.to_usize()].has_definition(entry.entry) {
                    let i = self.indices[INDEX_OF_INDICES_INDEX].resolve(entry.index);
                    let e = self.indices[entry.index.to_usize()].resolve(entry.entry);
                    tt_error!(status, "reference to index entry `{}:{}` that does not have an assocated definition location", i, e);
                    n_failures += 1;
                }
            }
        }

        match n_failures {
            0 => Ok(()),
            1 => Err(anyhow!("1 unresolved index reference")),
            n => Err(anyhow!("{} unresolved index references", n)),
        }
    }

    pub fn load_user_indices(&mut self) -> Result<()> {
        // Hardcoding that we're running from the root directory!
        let entries = atry!(
            std::fs::read_dir("idx");
            ["unable to read directory `idx`"]
        );

        for entry in entries {
            let entry = entry?;

            if !entry.file_type()?.is_file() {
                continue;
            }

            if !entry
                .file_name()
                .to_str()
                .unwrap_or_default()
                .ends_with(".toml")
            {
                continue;
            }

            let path = entry.path();

            let mut f = atry!(
                File::open(&path);
                ["failed to open index definition file `{}`", path.display()]
            );

            let mut text = String::new();
            atry!(
                f.read_to_string(&mut text);
                ["failed to read index definition file `{}`", path.display()]
            );

            let rec: syntax::Index = atry!(
                toml::from_str(&text);
                ["failed to parse index definition file `{}` as TOML", path.display()]
            );

            // Finally we can actually deal with this item

            atry!(
                self.declare_index(&rec.index.name);
                ["failed to declare the index defined in file `{}`", path.display()]
            );
        }

        Ok(())
    }
}

impl Default for IndexCollection {
    fn default() -> Self {
        let mut inst = IndexCollection {
            indices: vec![Index::new(
                IndexId::try_from_usize(INDEX_OF_INDICES_INDEX).unwrap(),
            )],
            refs: Default::default(),
        };

        let id = inst.indices[INDEX_OF_INDICES_INDEX].reference(INDEX_OF_INDICES_NAME);
        assert_eq!(id.to_usize(), INDEX_OF_INDICES_INDEX);

        let id = inst.declare_index(INPUTS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), INPUTS_INDEX_INDEX);

        let id = inst.declare_index(OUTPUTS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), OUTPUTS_INDEX_INDEX);

        let id = inst.declare_index(FRAGMENTS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), FRAGMENTS_INDEX_INDEX);

        inst
    }
}

/// An entry in an index.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct IndexEntry {
    pub index: IndexId,
    pub entry: EntryId,
}

/// A location in the output, specified by an ouput path name and a URL fragment
/// within that output.
///
/// This type has essentially the same structure as IndexEntry, but the
/// semantics of the two fields are different.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct OutputLocation {
    pub output: EntryId,
    pub fragment: EntryId,
}

impl OutputLocation {
    pub fn new(output: IndexId, fragment: EntryId) -> Self {
        OutputLocation { output, fragment }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_entry_option_size() {
        assert_eq!(std::mem::size_of::<Option<IndexEntry>>(), 8);
    }

    #[test]
    fn output_location_option_size() {
        assert_eq!(std::mem::size_of::<Option<OutputLocation>>(), 8);
    }
}

mod syntax {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct Index {
        pub index: Header,
    }

    #[derive(Debug, Deserialize)]
    pub struct Header {
        pub name: String,
    }
}
