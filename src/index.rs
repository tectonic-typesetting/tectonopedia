// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

#![allow(unused)]

use sha2::Digest;
use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
};
use string_interner::{StringInterner, Symbol};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_error, tt_warning, StatusBackend};

use crate::{
    cache::{Cache, OpCacheData},
    holey_vec::HoleyVec,
    metadata::Metadatum,
    multivec::MultiVec,
    operation::{DigestComputer, OpOutputStream, PersistEntityIdent, RuntimeEntityIdent},
    tex_escape::encode_tex_to_string,
    worker_status::WorkerStatusBackend,
    InputId,
};

use string_interner::DefaultSymbol as EntryId;
pub type IndexId = EntryId;

#[derive(Debug, Default)]
struct Index {
    entries: StringInterner,
    locs: Vec<Option<OutputLocation>>,
    texts: Vec<Option<EntryText>>,
}

impl Index {
    /// Ensure that the specified name exists in the index.
    fn reference(&mut self, name: impl AsRef<str>) -> EntryId {
        let name = name.as_ref();
        self.entries.get_or_intern(name)
    }

    /// Ensure that the name exists in the index, and declare the location of
    /// its definition.
    ///
    /// The operation can fail if the name has already had its location defined,
    /// and this definition is for a different location. In that case, the error
    /// value is the location of the previous definition.
    fn define_loc(
        &mut self,
        name: impl AsRef<str>,
        loc: OutputLocation,
    ) -> Result<EntryId, OutputLocation> {
        let entry = self.reference(name);
        let eidx = entry.to_usize();

        // The Err case will always be Some because no error is returned if the
        // existing value is the default.
        if let Err(Some(prev_loc)) = self.locs.ensure_holey_slot_available(eidx) {
            if *prev_loc == loc {
                return Err(*prev_loc);
            }
        }

        self.locs[eidx] = Some(loc);
        Ok(entry)
    }

    /// Ensure that the name exists in the index and associate a textual
    /// representation with it.
    ///
    /// The operation can fail if the name has already had its text defined
    /// and this definition is different than the existing one. In that case,
    /// the error value is the pair of the previous text and the new text
    /// (since this function takes ownership of the argument).
    fn define_text(
        &mut self,
        name: impl AsRef<str>,
        text: EntryText,
    ) -> Result<EntryId, (EntryText, EntryText)> {
        let entry = self.reference(name);
        let eidx = entry.to_usize();

        // The Err case will always be Some because no error is returned if the
        // existing value is the default.
        if let Err(Some(prev_text)) = self.texts.ensure_holey_slot_available(eidx) {
            if *prev_text == text {
                return Err((prev_text.clone(), text));
            }
        }

        self.texts[eidx] = Some(text);
        Ok(entry)
    }

    /// Get the numeric ID associated with the given entry name, if it has been
    /// defined.
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
    fn has_location(&self, entry: EntryId) -> bool {
        self.locs.holey_slot_is_filled(entry.to_usize())
    }

    /// Return whether the specified entry has a defined textualization.
    fn has_text(&self, entry: EntryId) -> bool {
        self.texts.holey_slot_is_filled(entry.to_usize())
    }

    /// Return the definition location of the specified entry, if it has been
    /// defined.
    fn get_location(&self, entry: EntryId) -> Option<OutputLocation> {
        self.locs.get_holey_slot(entry.to_usize())
    }

    /// Return the definition text of the specified entry, if it has been
    /// defined.
    fn get_text(&self, entry: EntryId) -> Option<EntryText> {
        self.texts.get_holey_slot(entry.to_usize())
    }

    fn iter(&self) -> impl IntoIterator<Item = (EntryId, &str)> {
        self.entries.into_iter()
    }

    /// Serialize this index in CSV format.
    ///
    /// We sort by entry name to hopefully keep the outputs reproducible.
    fn write<W: Write>(
        &self,
        dest: W,
        outputs_index: &Index,
        fragments_index: &Index,
    ) -> Result<W> {
        let mut all: Vec<_> = self.entries.into_iter().collect();
        all.sort_by_key(|t| t.1);

        let mut w = csv::Writer::from_writer(dest);

        w.write_record(&[
            "entry",
            "loc_output",
            "loc_fragment",
            "text_tex",
            "text_plain",
        ])?;

        for (entry_id, entry_text) in all.drain(..) {
            let (loc_output, loc_fragment) = self
                .locs
                .get_holey_slot(entry_id.to_usize())
                .map(|loc| {
                    (
                        outputs_index.resolve(loc.output),
                        fragments_index.resolve(loc.fragment),
                    )
                })
                .unwrap_or(("", ""));

            let (text_tex, text_plain) = self
                .texts
                .get_holey_slot(entry_id.to_usize())
                .map(|etext| (etext.tex, etext.plain))
                .unwrap_or((String::new(), String::new()));

            let rec = &[entry_text, loc_output, loc_fragment, &text_tex, &text_plain];
            w.write_record(rec)?;
        }

        Ok(w.into_inner().map_err(|e| e.into_error())?)
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
    refs: MultiVec<IndexRef>,

    /// The tree root doesn't have to do with the indices as used in the text
    /// processing, but we also use the indices to manage input and output
    /// paths used by the build system, so it's convenient to have the root
    /// saved here.
    root: PathBuf,
}

pub const INDEX_OF_INDICES_NAME: &'static str = "ioi";
const INDEX_OF_INDICES_INDEX: usize = 0;

pub const INPUTS_INDEX_NAME: &'static str = "inputs";
const INPUTS_INDEX_INDEX: usize = 1;

pub const OUTPUTS_INDEX_NAME: &'static str = "outputs";
const OUTPUTS_INDEX_INDEX: usize = 2;

pub const OTHER_PATHS_INDEX_NAME: &'static str = "otherpaths";
const OTHER_PATHS_INDEX_INDEX: usize = 3;

pub const FRAGMENTS_INDEX_NAME: &'static str = "fragments";
const FRAGMENTS_INDEX_INDEX: usize = 4;

impl IndexCollection {
    pub fn new() -> Result<Self> {
        let root = crate::config::get_root()?;

        let mut inst = IndexCollection {
            indices: vec![Index::default()],
            refs: Default::default(),
            root,
        };

        let id = inst.indices[INDEX_OF_INDICES_INDEX].reference(INDEX_OF_INDICES_NAME);
        assert_eq!(id.to_usize(), INDEX_OF_INDICES_INDEX);

        let id = inst.declare_index(INPUTS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), INPUTS_INDEX_INDEX);

        let id = inst.declare_index(OUTPUTS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), OUTPUTS_INDEX_INDEX);

        let id = inst.declare_index(OTHER_PATHS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), OTHER_PATHS_INDEX_INDEX);

        let id = inst.declare_index(FRAGMENTS_INDEX_NAME).unwrap();
        assert_eq!(id.to_usize(), FRAGMENTS_INDEX_INDEX);

        Ok(inst)
    }

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

        self.indices.push(Index::default());
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
    ) -> Result<(IndexId, EntryId)> {
        let index = self.get_index(index)?;
        let entry = self.indices[index.to_usize()].reference(entry);
        Ok((index, entry))
    }

    pub fn define_loc_by_id(
        &mut self,
        index: IndexId,
        entry: impl AsRef<str>,
        loc: OutputLocation,
    ) -> Result<EntryId> {
        let entry = entry.as_ref();

        self.indices[index.to_usize()].define_loc(entry, loc).map_err(|prev_loc|
            anyhow!(
                "redefinition of entry location `{}` in index `{}`; previous was `{}{}`, new is `{}{}`",
                entry,
                self.indices[INDEX_OF_INDICES_INDEX].resolve(index),
                self.indices[OUTPUTS_INDEX_INDEX].resolve(prev_loc.output),
                self.indices[FRAGMENTS_INDEX_INDEX].resolve(prev_loc.fragment),
                self.indices[OUTPUTS_INDEX_INDEX].resolve(loc.output),
                self.indices[FRAGMENTS_INDEX_INDEX].resolve(loc.fragment),
            )
        )
    }

    pub fn define_loc(
        &mut self,
        index: impl AsRef<str>,
        entry: impl AsRef<str>,
        loc: OutputLocation,
    ) -> Result<EntryId> {
        let id = self.get_index(index)?;
        self.define_loc_by_id(id, entry, loc)
    }

    pub fn define_text(
        &mut self,
        index: impl AsRef<str>,
        entry: impl AsRef<str>,
        text: EntryText,
    ) -> Result<EntryId> {
        let index = self.get_index(index)?;
        let entry = entry.as_ref();

        self.indices[index.to_usize()]
            .define_text(entry, text)
            .map_err(|(prev_text, text)| {
                let (prev_ex, new_ex) = if prev_text.tex != text.tex {
                    (&prev_text.tex, &text.tex)
                } else {
                    (&prev_text.plain, &text.plain)
                };

                anyhow!(
                    "redefinition of entry text `{}` in index `{}`; previous was `{}`, new is `{}`",
                    entry,
                    self.indices[INDEX_OF_INDICES_INDEX].resolve(index),
                    prev_ex,
                    new_ex
                )
            })
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
        refs: impl IntoIterator<Item = IndexRef>,
    ) -> Result<()> {
        self.refs.add_extend(input.to_usize(), refs)
    }

    /// Validate all of the cross-references.
    pub fn validate_references(&self) -> Result<()> {
        // Multiple inputs might reference the same entry, of course. We need to
        // keep track of references for each input, though, to know which
        // resolutions to provide in pass 2, and checking these resolutions
        // should be very quick, so we don't bother trying to coalesce the
        // checks.
        let mut n_failures = 0;

        for (input_id, input_name) in self.indices[INPUTS_INDEX_INDEX].iter() {
            let mut status = WorkerStatusBackend::new(input_name);

            // We always define the refs for every input, so this lookup can
            // never fail.
            let refs = self.refs.lookup(input_id.to_usize()).unwrap();

            for entry in refs {
                let f = entry.flags;

                if (f & IndexRefFlag::NeedsLoc as u8) != 0
                    && !self.indices[entry.index.to_usize()].has_location(entry.entry)
                {
                    let i = self.indices[INDEX_OF_INDICES_INDEX].resolve(entry.index);
                    let e = self.indices[entry.index.to_usize()].resolve(entry.entry);
                    tt_error!(status, "reference to location of index entry `{}:{}` that does not have one defined", i, e);
                    n_failures += 1;
                }

                if (f & IndexRefFlag::NeedsText as u8) != 0
                    && !self.indices[entry.index.to_usize()].has_text(entry.entry)
                {
                    let i = self.indices[INDEX_OF_INDICES_INDEX].resolve(entry.index);
                    let e = self.indices[entry.index.to_usize()].resolve(entry.entry);
                    tt_error!(
                        status,
                        "reference to text of index entry `{}:{}` that does not have it defined",
                        i,
                        e
                    );
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

    /// Get a user-friendly(ish) summary of the indexing data.
    pub fn index_summary(&self) -> String {
        let n_indices = self.indices.len();

        let mut n_entries = 0;

        for idx in &self.indices {
            n_entries += idx.entries.len();
        }

        format!("{n_entries} entries in {n_indices} indices")
    }

    pub fn get_resolved_reference_tex(&self, input: InputId) -> String {
        // Because we have validated cross-references, we can unwrap everything
        // here without worrying about missing values.
        let refs = self.refs.lookup(input.to_usize()).unwrap();
        let mut tex = String::new();

        for entry in refs {
            let iname = self.indices[INDEX_OF_INDICES_INDEX].resolve(entry.index);
            let iindex = entry.index.to_usize();
            let ename = self.indices[iindex].resolve(entry.entry);
            let f = entry.flags;

            if (f & IndexRefFlag::NeedsLoc as u8) != 0 {
                let loc = self.indices[iindex].get_location(entry.entry).unwrap();
                let o = self.indices[OUTPUTS_INDEX_INDEX].resolve(loc.output);
                let f = self.indices[FRAGMENTS_INDEX_INDEX].resolve(loc.fragment);

                let o = if o.ends_with("/index.html") {
                    &o[..o.len() - 10]
                } else {
                    o
                };

                writeln!(
                    tex,
                    r"\expandafter\def\csname pedia resolve**{}**{}**loc\endcsname{{{}{}}}",
                    iname, ename, o, f
                )
                .unwrap();
            }

            if (f & IndexRefFlag::NeedsText as u8) != 0 {
                let text = self.indices[iindex].get_text(entry.entry).unwrap();
                writeln!(
                    tex,
                    r"\expandafter\def\csname pedia resolve**{}**{}**text tex\endcsname{{{}}}",
                    iname, ename, text.tex
                )
                .unwrap();
                write!(
                    tex,
                    r"\expandafter\def\csname pedia resolve**{}**{}**text plain\endcsname{{",
                    iname, ename
                )
                .unwrap();
                encode_tex_to_string(text.plain, &mut tex);
                writeln!(tex, r"}}",).unwrap();
            }
        }

        tex
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

    // Support for the "operation" framework

    /// Create an entity identifier for a TeX source input file.
    ///
    /// This returns a value of [`RuntimeEntityIdent::TexSourceFile`].
    #[inline(always)]
    pub fn make_tex_source_ident(&mut self, relpath: impl AsRef<str>) -> RuntimeEntityIdent {
        let id = self.indices[INPUTS_INDEX_INDEX].reference(relpath);
        RuntimeEntityIdent::TexSourceFile(id)
    }

    /// Create an entity identifier for an output file.
    ///
    /// This returns a value of [`RuntimeEntityIdent::OutputFile`].
    #[inline(always)]
    pub fn make_output_file_ident(&mut self, relpath: impl AsRef<str>) -> RuntimeEntityIdent {
        let id = self.indices[OUTPUTS_INDEX_INDEX].reference(relpath);
        RuntimeEntityIdent::OutputFile(id)
    }

    /// Create an entity identifier for a file not matching one of the
    /// other categories.
    ///
    /// This returns a value of [`RuntimeEntityIdent::OtherFile`].
    #[inline(always)]
    pub fn make_other_file_ident(&mut self, relpath: impl AsRef<str>) -> RuntimeEntityIdent {
        let id = self.indices[OTHER_PATHS_INDEX_INDEX].reference(relpath);
        RuntimeEntityIdent::OtherFile(id)
    }

    /// Convert a [`RuntimeEntityIdent`] to a [`PersistEntityIdent`].
    pub fn persist_ident(&self, rei: RuntimeEntityIdent) -> PersistEntityIdent {
        match rei {
            RuntimeEntityIdent::TexSourceFile(s) => {
                let p = self.indices[INPUTS_INDEX_INDEX].resolve(s);
                PersistEntityIdent::TexSourceFile(p.to_owned())
            }

            RuntimeEntityIdent::OutputFile(s) => {
                let p = self.indices[OUTPUTS_INDEX_INDEX].resolve(s);
                PersistEntityIdent::OutputFile(p.to_owned())
            }

            RuntimeEntityIdent::OtherFile(s) => {
                let p = self.indices[OTHER_PATHS_INDEX_INDEX].resolve(s);
                PersistEntityIdent::OtherFile(p.to_owned())
            }
        }
    }

    /// Get a filesystem path associated with a [`PersistEntityIdent`], if one
    /// exists.
    pub(crate) fn path_for_runtime_ident(&self, rei: RuntimeEntityIdent) -> Result<PathBuf> {
        let p = match rei {
            RuntimeEntityIdent::TexSourceFile(s) => {
                let mut p = PathBuf::new();
                p.push(self.indices[INPUTS_INDEX_INDEX].resolve(s));
                p
            }

            RuntimeEntityIdent::OutputFile(s) => {
                let mut p = self.root.clone();
                p.push("staging");
                p.push(self.indices[OUTPUTS_INDEX_INDEX].resolve(s));
                p
            }

            RuntimeEntityIdent::OtherFile(s) => {
                let mut p = self.root.clone();
                p.push(self.indices[OTHER_PATHS_INDEX_INDEX].resolve(s));
                p
            }
        };

        /// One day, we may have idents that don't have associated paths, but
        /// right now, they all do.
        Ok(p)
    }

    /// Get a relative path associated with a TeX source file.
    ///
    /// This is a specialized helper for formatting nice outputs.
    pub(crate) fn relpath_for_tex_source(&self, rei: RuntimeEntityIdent) -> Option<&str> {
        match rei {
            RuntimeEntityIdent::TexSourceFile(s) => {
                Some(self.indices[INPUTS_INDEX_INDEX].resolve(s))
            }
            _ => None,
        }
    }

    /// Convert a [`PersistEntityIdent`] to a [`RuntimeEntityIdent`].
    pub fn runtime_ident(&mut self, pei: &PersistEntityIdent) -> RuntimeEntityIdent {
        match pei {
            PersistEntityIdent::TexSourceFile(p) => self.make_tex_source_ident(p),
            PersistEntityIdent::OutputFile(p) => self.make_output_file_ident(p),
            PersistEntityIdent::OtherFile(p) => self.make_other_file_ident(p),
        }
    }
}

/// An reference to an entry in an index.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct IndexRef {
    pub index: IndexId,
    pub entry: EntryId,
    pub flags: IndexRefFlags,
}

/// A location in the output, specified by an ouput path name and a URL fragment
/// within that output.
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

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct EntryText {
    /// Some text in TeX markup, suitable for direct insertion into TeX source
    /// code. E.g., `"\\TeX \\& \\LaTeX"`.
    pub tex: String,

    /// The "plain" equivalent of the text, without any control sequences or
    /// escaping. E.g., `"TeX & LaTeX"`.
    pub plain: String,
}

pub type IndexRefFlags = u8;

#[repr(u8)]
pub enum IndexRefFlag {
    NeedsLoc = 1 << 0,
    NeedsText = 1 << 1,
}

// The index construction phase of the build

pub fn maybe_indexing_operation(
    indices: &mut IndexCollection,
    metadata_ids: &[RuntimeEntityIdent],
    cache: &mut Cache,
    status: &mut dyn StatusBackend,
) -> Result<()> {
    // Set up the information about the operation. The operation identifier
    // must include *all* inputs since if, say, we add a new one, we'll need
    // to rerun the op, and a simple check that all of the old inputs are
    // unchanged won't catch that.

    let mut dc = DigestComputer::default();
    dc.update("internal_index_v2");

    for input in metadata_ids {
        input.update_digest(&mut dc, indices);
    }

    let opid = dc.finalize();

    let needs_rerun = atry!(
        cache.operation_needs_rerun(&opid, indices, status);
        ["failed to probe cache for internal indexing operation"]
    );

    if !needs_rerun {
        return Ok(());
    }

    // It seems that we need to rerun the indexing.

    let mut ocd = OpCacheData::new(opid);

    for input in metadata_ids {
        ocd.add_input(*input);

        let (input_id, index_refs) = atry!(
            load_metadata(*input, indices, cache, status);
            ["failed to load metadata for `{:?}`", input]
        );

        // This function only fails if the references for the given input have
        // already been logged, which should never happen to us.
        indices.log_references(input_id, index_refs).unwrap();
    }

    atry!(
        indices.validate_references();
        ["failed to validate cross-references"]
    );

    // Yay, indices are good. Write them to disk. First we need to create the
    // entity IDs for our output files, since doing so requires a mutable ref to
    // the indices.

    let mut index_file_names: Vec<String> = indices.indices[INDEX_OF_INDICES_INDEX]
        .iter()
        .into_iter()
        .map(|t| format!("cache/idx/{}.csv", t.1))
        .collect();

    let index_files: Vec<_> = index_file_names
        .drain(..)
        .map(|p| RuntimeEntityIdent::new_other_file(p, indices))
        .collect();

    // Now we can write everything out.

    let ioi_index = &indices.indices[INDEX_OF_INDICES_INDEX];
    let outputs_index = &indices.indices[OUTPUTS_INDEX_INDEX];
    let fragments_index = &indices.indices[FRAGMENTS_INDEX_INDEX];

    for (index_id, index_name) in ioi_index.iter() {
        let index_id: EntryId = index_id; // need this to de-confuse rust-analyzer

        // Skip internal indices that won't be used directly by TeX
        // files and should be rebuilt properly on the fly.

        match index_name {
            "ioi" | "otherpaths" | "fragments" => continue,
            _ => {}
        }

        // OK, we should write this one out.

        let index_id = index_id.to_usize();
        let index = &indices.indices[index_id];
        let stream = OpOutputStream::new(index_files[index_id], indices)?;
        let mut stream = index.write(stream, outputs_index, fragments_index)?;
        let (entity, size) = stream.close()?;
        ocd.add_output_with_value(index_files[index_id], entity.value_digest, size);
    }

    // Cache it and we're done!

    atry!(
        cache.finalize_operation(ocd, indices);
        ["failed to store caching information for indexing operation"]
    );

    Ok(())
}

fn load_metadata(
    input: RuntimeEntityIdent,
    indices: &mut IndexCollection,
    cache: &mut Cache,
    status: &mut dyn StatusBackend,
) -> Result<(InputId, impl IntoIterator<Item = IndexRef>)> {
    let outputs_id = indices.get_index("outputs").unwrap();
    let mut cur_output = None;
    let mut index_refs = HashMap::new();

    let meta_path = indices.path_for_runtime_ident(input).unwrap();

    let meta_file = atry!(
        File::open(&meta_path);
        ["failed to open input `{}`", meta_path.display()]
    );

    let mut meta_buf = BufReader::new(meta_file);

    let mut context = String::new();
    atry!(
        meta_buf.read_line(&mut context);
        ["failed to read input `{}`", meta_path.display()]
    );

    // Note: read_line() includes the trailing newline!
    let input_id = if let Some(input_path) = context.strip_prefix("% input ") {
        // This can only fail if the index name is undefined, which is impossible here.
        indices
            .reference(INPUTS_INDEX_NAME, input_path.trim_end())
            .unwrap()
    } else {
        bail!(
            "unexpected first line of metadata file `{}`",
            meta_path.display()
        );
    };

    for line in meta_buf.lines() {
        let line = atry!(
            line;
            ["failed to read input `{}`", meta_path.display()]
        );

        match Metadatum::parse(&line)? {
            Metadatum::Output(path) => {
                // TODO: make sure there are no redundant outputs
                cur_output = Some(indices.reference_by_id(outputs_id, path));
            }

            Metadatum::IndexDef {
                index,
                entry,
                fragment,
            } => {
                if let Err(e) = indices.reference(index, entry) {
                    tt_warning!(status, "couldn't define entry `{}` in index `{}`", entry, index; e);
                    continue;
                }

                let co = match cur_output.as_ref() {
                    Some(o) => *o,
                    None => {
                        tt_warning!(status, "attempt to define entry `{}` in index `{}` before an output has been specified", entry, index);
                        continue;
                    }
                };

                let loc = indices.make_location_by_id(co, fragment);

                if let Err(e) = indices.define_loc(index, entry, loc) {
                    // The error here will contain the contextual information.
                    tt_warning!(status, "couldn't define an index entry"; e);
                }
            }

            Metadatum::IndexRef {
                index,
                entry,
                flags,
            } => {
                let ie = match indices.reference_to_entry(index, entry) {
                    Ok(ie) => ie,

                    Err(e) => {
                        tt_warning!(status, "couldn't reference entry `{}` in index `{}`", entry, index; e);
                        continue;
                    }
                };

                let cur_flags = index_refs.entry(ie).or_default();
                *cur_flags |= flags;
            }

            Metadatum::IndexText {
                index,
                entry,
                tex,
                plain,
            } => {
                if let Err(e) = indices.reference(index, entry) {
                    tt_warning!(status, "couldn't define entry `{}` in index `{}`", entry, index; e);
                    continue;
                }

                let text = EntryText {
                    tex: tex.to_owned(),
                    plain: plain.to_owned(),
                };

                if let Err(e) = indices.define_text(index, entry, text) {
                    // The error here will contain the contextual information.
                    tt_warning!(status, "couldn't define the text of an index entry"; e);
                }
            }
        }
    }

    Ok((
        input_id,
        index_refs
            .into_iter()
            .map(|((index, entry), flags)| IndexRef {
                index,
                entry,
                flags,
            }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
