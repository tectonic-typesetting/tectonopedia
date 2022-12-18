// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! A helper trait for managing vectors that grow irregularly.

pub trait HoleyVec {
    type Item;

    /// Ensure that the vec is able to be indexed with the given index. If the
    /// vec contains a value at the index that is not the default, an error is
    /// returned with a mutable reference to the existing entry.
    fn ensure_holey_slot_available(&mut self, index: usize) -> Result<(), &mut Self::Item>;

    /// Get the value at the specified slot, or some kind of default if it has
    /// not been entered into the vec.
    fn get_holey_slot(&self, index: usize) -> Self::Item;

    /// Test whether the specified slot is filled with a non-default value.
    fn holey_slot_is_filled(&self, index: usize) -> bool;
}

impl<T: Clone + Default + PartialEq> HoleyVec for Vec<T> {
    type Item = T;

    fn ensure_holey_slot_available(&mut self, index: usize) -> Result<(), &mut Self::Item> {
        if self.len() > index {
            if self[index] != Self::Item::default() {
                return Err(&mut self[index]);
            }
        } else {
            self.resize(index + 1, Self::Item::default());
        }

        Ok(())
    }

    fn get_holey_slot(&self, index: usize) -> Self::Item {
        if self.len() <= index {
            Self::Item::default()
        } else {
            self[index].clone()
        }
    }

    fn holey_slot_is_filled(&self, index: usize) -> bool {
        if self.len() <= index {
            false
        } else {
            self[index] != Self::Item::default()
        }
    }
}
