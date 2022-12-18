// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! A simple arena-like data structure for a group of vectors, all of varying
//! sizes, but sharing the same underlying data type, and with each element of
//! the group identified by usizes that are densely packed around zero.

use tectonic_errors::prelude::*;

use crate::holey_vec::HoleyVec;

#[derive(Clone, Debug)]
pub struct MultiVec<T> {
    /// All of the data that have been added to the multi-vec, in chunks in the
    /// order in which they were added.
    buffer: Vec<T>,

    /// Offsets into *buffer*, one for each ID, but in the order that they were
    /// added, which may not correspond to the order of the actual IDs. The
    /// size of each ID's slice of the buffer is determined from the
    /// difference between its offset and the next one in this vector.
    inner_indices: Vec<usize>,

    /// Mapping from the IDs used by the caller to indices into the
    /// `inner_indices` array ... PLUS ONE. The special value zero means that
    /// the ID has not been added into the multi-vec yet.
    outer_indices: Vec<usize>,
}

impl<T> MultiVec<T> {
    /// Add a new entry to the multi-vec. Returns an error if it's already been
    /// added.
    pub fn add_extend<I>(&mut self, id: usize, iter: I) -> Result<()>
    where
        I: IntoIterator<Item = T>,
    {
        if self.outer_indices.ensure_holey_slot_available(id).is_err() {
            bail!("id {} has already been added to the multi-vec", id);
        }

        let ii = self.inner_indices.len();
        self.outer_indices[id] = ii + 1;
        let bi = self.buffer.len();
        self.inner_indices.push(bi);
        self.buffer.extend(iter);
        Ok(())
    }

    pub fn lookup(&self, id: usize) -> Result<&[T]> {
        let ii1 = self.outer_indices.get_holey_slot(id);

        if ii1 == 0 {
            bail!("id {} has not been added to the multi-vec yet", id);
        }

        let bi0 = self.inner_indices[ii1 - 1];

        Ok(if ii1 == self.inner_indices.len() {
            // This is currently the last-added item to the multi-vec.
            &self.buffer[bi0..]
        } else {
            let bi1 = self.inner_indices[ii1];
            &self.buffer[bi0..bi1]
        })
    }
}

// Not sure why I have to declare this manually? The derived default impl needs
// T to be Default as well.
impl<T> Default for MultiVec<T> {
    fn default() -> Self {
        MultiVec {
            buffer: Default::default(),
            inner_indices: Default::default(),
            outer_indices: Default::default(),
        }
    }
}
