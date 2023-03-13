// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Build operations.
//!
//! This formalism for defining elements of the build process allows us to
//! cache results and provide incremental builds.
//!
//! The key concepts are as follows:
//!
//! - An **entity** is some thing that is a potential input or output of a build
//!   operation. Most entities correspond to files on the filesystem, but other
//!   entity types are possible (e.g., a build operation might depend on an
//!   environment variable, in the sense that the operation might produce a
//!   different output if the variable changes).
//! - A **digest** is a cryptographic digest of some byte sequence.
//! - An **identity** uniquely identifies an entity. You can compute the digest of
//!   an identity.
//! - Each entity may have a **value** representing its current state. You can also
//!   compute the digest of a value.
//! - An **operation** is a build operation that generates output entities from
//!   input entities. If an operation is run repeatedly with the same inputs, it
//!   should generate the same outputs.

use bincode;
use digest::OutputSizeUser;
use generic_array::GenericArray;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, ErrorKind, Write},
    path::PathBuf,
};
use tectonic_errors::{anyhow::Context, prelude::*};
use tempfile::NamedTempFile;

use crate::{index::IndexCollection, InputId};

/// A type used to compute data digests for change detection.
///
/// This is currently [`sha2::Sha256`].
pub type DigestComputer = Sha256;

/// The data type emitted by [`DigestComputer`].
///
/// This is a particular form of [`generic_array::GenericArray`] with a [`u8`]
/// data type and a size appropriate to the digest. For the current SHA256
/// implementation, that's 32 bytes.
pub type DigestData = GenericArray<u8, <DigestComputer as OutputSizeUser>::OutputSize>;

/// A [`string_interner`] "symbol" used for our various paths.
type PathId = InputId;

/// The unique identifier of a logical entity that can be an input or an output
/// of a build operation, as can be serialized to persistent storage.
///
/// See also [`RuntimeEntityIdent`], in which string values have been interned
/// into ids.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PersistEntityIdent {
    /// A TeX input file. The string value is the path to the file in question,
    /// relative to the `txt` tree.
    ///
    /// These inputs are handled specially so that they can map 1:1 to the "inputs"
    /// index maintained by an [`IndexCollection`].
    TexSourceFile(String),

    /// A file that does not belong to one of the other categories. Its path is
    /// relative to the project root.
    OtherFile(String),
}

impl PersistEntityIdent {
    fn update_digest(&self, dc: &mut DigestComputer) {
        let data = bincode::serialize(self).unwrap(); // can this ever realistically fail?
        dc.update(data);
    }

    //fn compute_digest(&self) -> DigestData {
    //    let mut dc = DigestComputer::new();
    //    let data = bincode::serialize(self).unwrap(); // can this ever realistically fail?
    //    dc.update(data);
    //    dc.finalize()
    //}

    /// Get a filesystem path associated with this identity, if one exists.
    ///
    /// The corresponding function on [`RuntimeEntityIdent`] is attached to the
    /// [`IndexCollection`] type because it depends on its internal structure.
    pub fn path(&self, root: PathBuf) -> Result<PathBuf> {
        let mut p = root;

        match self {
            PersistEntityIdent::TexSourceFile(relpath) => {
                p.push("txt");
                p.push(relpath);
            }

            PersistEntityIdent::OtherFile(relpath) => {
                p.push(relpath);
            }
        }

        Ok(p)
    }

    /// Test whether the external artifact corresponding to this identity exists
    /// in the environment.
    ///
    /// By "exists", we mean that processes that depend on it could access it in
    /// whatever way they expect, including our ability to calculate its digest.
    /// For identities corresponding to files, that simply means that the file
    /// exists. As with all such tests, we should keep in mind that there is
    /// potentially a race condition between this function and any subsequent
    /// attempts to actually access the artifact.
    pub fn artifact_exists(&self, root: PathBuf) -> Result<bool> {
        // For now, all we have are files, so:
        let p = self.path(root).unwrap();

        match fs::metadata(&p) {
            Ok(_) => Ok(true),
            Err(ref e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).context(format!("failed to probe path `{}`", p.display())),
        }
    }
}

/// For our purposes, an entity is a tuple of its identity and the digest of
/// its value. This form is one that can be serialized to persistent storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PersistEntity {
    /// The identity of the entity.
    pub ident: PersistEntityIdent,

    /// The digest of the entity's value.
    pub value_digest: DigestData,
}

impl PersistEntity {
    /// Return the runtime equivalent of this entity.
    pub fn as_runtime(&self, indices: &mut IndexCollection) -> RuntimeEntity {
        RuntimeEntity {
            ident: indices.runtime_ident(&self.ident),
            value_digest: self.value_digest.clone(),
        }
    }
}

/// The unique identifier of a logical entity that can be an input or an output
/// of a build operation, as managed during runtime. String values are interned
/// into ids.
///
/// See also [`PersistEntityIdent`], in which interned string Ids have been
/// expanded into owned strings. That type can be serialized and deserialized,
/// whereas this type implements [`std::marker::Copy`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeEntityIdent {
    /// A source file. The id resolves to the path to the file in question,
    /// relative to the project root.
    TexSourceFile(InputId),

    /// A file that does not belong to one of the other categories. The id
    /// resolves to a path relative to the project root.
    OtherFile(PathId),
}

impl RuntimeEntityIdent {
    /// Create a new entity for a TeX source entity.
    ///
    /// This needs to go through the index collection to potentially register
    /// the relative source path with an index.
    pub fn new_tex_source(relpath: impl AsRef<str>, indices: &mut IndexCollection) -> Self {
        indices.make_tex_source_ident(relpath)
    }

    /// Create a new entity for a file entity that doesn't fit one of the other
    /// categories.
    ///
    /// This needs to go through the index collection to potentially register
    /// the relative source path with an index.
    pub fn new_other_file(relpath: impl AsRef<str>, indices: &mut IndexCollection) -> Self {
        indices.make_other_file_ident(relpath)
    }

    /// Add information about this identity to a [`DigestComputer`].
    ///
    /// This is accomplished by computing the "persistent" version of this
    /// identity, since the digest should be repeatable across different
    /// invocations of the program.
    pub fn update_digest(&self, dc: &mut DigestComputer, indices: &IndexCollection) {
        indices.persist_ident(*self).update_digest(dc);
    }
}

/// For our purposes, an entity is a tuple of its identity and the digest of
/// its value. This form is the one used during runtime.
#[derive(Clone, Debug)]
pub struct RuntimeEntity {
    /// The identity of the entity.
    pub ident: RuntimeEntityIdent,

    /// The digest of the entity's value.
    pub value_digest: DigestData,
}

impl RuntimeEntity {
    /// Return the persistable equivalent of this entity.
    pub fn as_persist(&self, indices: &IndexCollection) -> PersistEntity {
        PersistEntity {
            ident: indices.persist_ident(self.ident),
            value_digest: self.value_digest.clone(),
        }
    }
}

/// A helper for creating build output files that are streamed to disk.
///
/// This class calculates the cryptographic digest as the data are written, so
/// that it can be cached efficiently. It also uses a temporary file with an
/// atomic rename upon build completion so that partially-created outputs are
/// not observed.
#[derive(Debug)]
pub struct OpOutputStream {
    ident: RuntimeEntityIdent,
    path: PathBuf,
    file: NamedTempFile,
    dc: DigestComputer,
    size: u64,
}

impl OpOutputStream {
    /// Create a new output stream.
    pub fn new(ident: RuntimeEntityIdent, indices: &IndexCollection) -> Result<Self> {
        let path = indices.path_for_runtime_ident(ident)?;

        let file = if let Some(dir) = path.parent() {
            atry!(
                fs::create_dir_all(dir);
                ["failed to create directory tree `{}`", dir.display()]
            );

            atry!(
                NamedTempFile::new_in(dir);
                ["failed to create temporary file `{}`", dir.display()]
            )
        } else {
            // This should never happen, but might as well be paranoid.
            atry!(
                NamedTempFile::new();
                ["failed to create a temporary file"]
            )
        };

        let dc = DigestComputer::new();

        Ok(OpOutputStream {
            ident,
            path,
            file,
            dc,
            size: 0,
        })
    }

    /// Close the stream and return an entity corresponding to its
    /// contents.
    ///
    /// This consumes the object.
    ///
    /// This operation uses standard Rust drop semantics to close the output
    /// file, and so cannot detect any I/O errors that occur as the file is
    /// closed.
    pub fn close(mut self) -> Result<RuntimeEntity> {
        atry!(
            self.flush();
            ["failed to flush file `{}`", self.path.display()]
        );

        let path = self.path;

        atry!(
            self.file.persist(&path);
            ["failed to persist temporary file to `{}`", path.display()]
        );

        let value_digest = self.dc.finalize();

        Ok(RuntimeEntity {
            ident: self.ident,
            value_digest,
        })
    }

    /// Get a displayable form of the path of this file.
    pub fn display_path(&self) -> std::path::Display {
        self.path.display()
    }
}

impl io::Write for OpOutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // If the actual write to disk is short, make sure our digest honors
        // that.
        let size = self.file.write(buf)?;
        self.dc.write(&buf[..size])?;
        self.size += size as u64;
        Ok(size)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.dc.flush()?;
        self.file.flush()
    }
}
