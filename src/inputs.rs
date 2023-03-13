// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use tectonic_errors::prelude::*;
use walkdir::{DirEntry, Error as WalkDirError, WalkDir};

use crate::{index::IndexCollection, operation::RuntimeEntityIdent};

pub struct InputIterator {
    inner: Box<dyn Iterator<Item = Result<DirEntry, WalkDirError>>>,
}

impl InputIterator {
    pub fn new() -> Self {
        // Hardcoding that we're running from the root directory!
        let inner = Box::new(WalkDir::new("txt").into_iter().filter_entry(is_tex_or_dir));
        InputIterator { inner }
    }
}

impl Iterator for InputIterator {
    type Item = Result<DirEntry>;

    fn next(&mut self) -> Option<Result<DirEntry>> {
        loop {
            let entry = self.inner.next()?;

            match entry {
                Ok(e) => {
                    if !e.file_type().is_dir() {
                        return Some(Ok(e));
                    }
                }

                Err(e) => return Some(Err(e.into())),
            }
        }
    }
}

/// Collect all input paths into a vector of strings.
pub fn collect_inputs(indices: &mut IndexCollection) -> Result<Vec<RuntimeEntityIdent>> {
    let mut paths = Vec::new();

    for entry in InputIterator::new() {
        let entry = atry!(
            entry;
            ["error while walking input tree"]
        );

        let entry = a_ok_or!(
            entry.path().to_str();
            ["input paths must be Unicode-compatible; failed with `{}`", entry.path().display()]
        );

        paths.push(RuntimeEntityIdent::new_tex_source(entry, indices));
    }

    Ok(paths)
}

fn is_tex_or_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        || entry
            .file_name()
            .to_str()
            .map(|s| s.ends_with(".tex") && !s.starts_with('_'))
            .unwrap_or(false)
}
