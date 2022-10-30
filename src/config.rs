// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use std::{env::current_dir, path::PathBuf};
use tectonic_errors::prelude::*;

pub fn get_root() -> Result<PathBuf> {
    Ok(current_dir()?)
}
