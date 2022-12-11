// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use std::{fs::File, io::Read};
use string_interner::StringInterner;
use tectonic_errors::prelude::*;

use string_interner::{DefaultSymbol as EntryId, Symbol};
pub type IndexId = EntryId;

#[derive(Debug)]
struct Index {
    entries: StringInterner,
}

impl Index {
    fn new(_id: IndexId) -> Self {
        Index {
            entries: Default::default(),
        }
    }

    fn define(&mut self, name: impl AsRef<str>) -> EntryId {
        self.entries.get_or_intern(name)
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
}

#[derive(Debug)]
pub struct IndexCollection {
    indices: Vec<Index>,
}

pub const INDEX_OF_INDICES_NAME: &'static str = "ioi";
const INDEX_OF_INDICES_INDEX: usize = 0;

impl IndexCollection {
    pub fn declare_index(&mut self, name: impl AsRef<str>) -> Result<IndexId> {
        let name = name.as_ref();
        let id: IndexId = self.indices[INDEX_OF_INDICES_INDEX].define(name);
        let idx = id.to_usize();

        if idx != self.indices.len() {
            bail!("re-declaration of index `{}`", name);
        }

        self.indices.push(Index::new(id));
        Ok(id)
    }

    pub fn get_index(&self, name: impl AsRef<str>) -> Result<IndexId> {
        let name = name.as_ref();
        Ok(a_ok_or!(
            self.indices[INDEX_OF_INDICES_INDEX].get(name);
            ["no such index `{}`", name]
        ))
    }

    pub fn define_by_id(&mut self, index: IndexId, entry: impl AsRef<str>) -> EntryId {
        self.indices[index.to_usize()].define(entry)
    }

    pub fn define(
        &mut self,
        index_name: impl AsRef<str>,
        entry: impl AsRef<str>,
    ) -> Result<EntryId> {
        let id = self.get_index(index_name)?;
        Ok(self.indices[id.to_usize()].define(entry))
    }

    pub fn resolve_by_id(&self, index: IndexId, entry: EntryId) -> &str {
        self.indices[index.to_usize()].resolve(entry)
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
        };

        let id = inst.indices[INDEX_OF_INDICES_INDEX].define(INDEX_OF_INDICES_NAME);
        assert!(id.to_usize() == INDEX_OF_INDICES_INDEX);
        inst
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
