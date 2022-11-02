// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use tectonic_errors::prelude::*;
use walkdir::{DirEntry, Error as WalkDirError, WalkDir};

pub struct InputIterator {
    inner: Box<dyn Iterator<Item = Result<DirEntry, WalkDirError>>>,
}

impl InputIterator {
    pub fn new() -> Self {
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

fn is_tex_or_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        || entry
            .file_name()
            .to_str()
            .map(|s| s.ends_with(".tex"))
            .unwrap_or(false)
}
