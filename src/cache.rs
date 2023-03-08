// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Caching infrastructure for incremental builds.
//!
//! We don't try to implement a full dependency graph structure since our build
//! process is fairly simple. But, the infrastructure for checking whether
//! specific build operations need to be rerun aims to be flexible so that it
//! can capture lots of different steps.
//!
//! The key concepts are as follows:
//!
//! - A **digest** is a cryptographic digest of some byte sequence. Changes in
//!   dependencies are detected by searching for changes in their digests.
//! - An **entity** is some thing that is a potential input or output of a build
//!   operation. Most entities correspond to files on the filesystem, but other
//!   entity types are possible (e.g., a build operation might depend on an
//!   environment variable, in the sense that the operation might produce a
//!   different output if the variable changes). Each entity has an
//!   **identity**, which uniquely identifies it.
//! - An **instance** of an entity is a combination of its identity and a
//!   **digest** representing its value.
//! - An **operation** is a build operation that generates output entities from
//!   input entities. If an operation is run repeatedly with the same inputs, it
//!   should generate the same outputs.

#![allow(unused)]

use bincode;
use digest::OutputSizeUser;
use generic_array::GenericArray;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use string_interner::{DefaultSymbol, StringInterner};
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_warning, StatusBackend};
use tempfile::NamedTempFile;

use crate::config;

/// A type that we'll use to compute data digests for change detection.
///
/// This is currently [`sha2::Sha256`].
pub type DigestComputer = Sha256;

/// The data type emitted by [`DigestComputer`].
///
/// This is a particular form of [`generic_array::GenericArray`] with a [`u8`]
/// data type and a size appropriate to the digest. For the current SHA256
/// implementation, that's 32 bytes.
pub type DigestData = GenericArray<u8, <DigestComputer as OutputSizeUser>::OutputSize>;

/// A [`string_interner`] symbol used by the [`Cache`] interner for paths.
type PathSymbol = DefaultSymbol;

/// Helper for caching file digests based on modification times.
///
/// Most of our build inputs and outputs are files on disk. It would get pretty
/// slow to recalculate the digest of their contents every time we needed to
/// check if they've changed. So, we do what virtually every other build system
/// does, and we cache file digests based on their filesystem metadata,
/// specifically modification times and sizes. If a files mtime and size are
/// what we have in our cache, we assume that its digest is as well.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct FileDigestEntry {
    digest: DigestData,
    mtime: SystemTime,
    size: u64,
}

/// Calculate the digest and size of a file, reading the whole thing.
fn digest_of_file(p: impl AsRef<Path>) -> Result<(u64, DigestData)> {
    // We could get the file size from the filesystem metadata, but as long
    // as we have to read the whole thing, it seems better to use the size
    // that we get from the streaming operation, I think?

    let mut f = fs::File::open(p)?;
    let mut dc = DigestComputer::new();
    let size = io::copy(&mut f, &mut dc)?;
    let digest = dc.finalize();
    Ok((size, digest))
}

impl FileDigestEntry {
    /// Create an entirely new file digest cache entry by reading the whole
    /// file.
    pub fn create(p: impl AsRef<Path>) -> Result<FileDigestEntry> {
        let p = p.as_ref();
        let md = fs::metadata(p)?;
        let mtime = md.modified()?;
        let (size, digest) = digest_of_file(p)?;

        Ok(FileDigestEntry {
            digest,
            mtime,
            size,
        })
    }

    /// Make sure that the information associated with this cache entry is
    /// fresh.
    ///
    /// If the mtime and size of the file at the specified path are the same as
    /// what's been saved, assume that the file is unchanged and we don't need
    /// to update the digest. Otherwise, recalculate the digest.
    pub fn freshen(&mut self, p: impl AsRef<Path>) -> Result<()> {
        // TODO: do we need to do something special for ENOENT here?
        let p = p.as_ref();
        let md = fs::metadata(p)?;
        let mtime = md.modified()?;

        if mtime != self.mtime || md.len() != self.size {
            let (new_size, new_digest) = digest_of_file(p)?;
            self.size = new_size;
            self.digest = new_digest;
        }

        Ok(())
    }

    /// Create an entry for a while whose digest we're absolutely sure that we
    /// know; that is, a file that we've just created and closed.
    ///
    /// Since we need to get the file mtime from a metadata call anyway, we
    /// compare the expected size to the one on disk to try to detect any funny
    /// stuff. It is of course possible that someone has modified the file
    /// between close and the execution of this function in a way that means
    /// that our digest will actually be wrong, even though the size is the
    /// same.
    pub fn create_for_known(
        p: impl AsRef<Path>,
        digest: DigestData,
        size: u64,
    ) -> Result<FileDigestEntry> {
        let p = p.as_ref();
        let md = fs::metadata(p)?;
        let mtime = md.modified()?;
        let actual_size = md.len();

        if actual_size != size {
            bail!("error saving digest of new file: expected file size of {size} but found {actual_size}");
        }

        Ok(FileDigestEntry {
            digest,
            mtime,
            size,
        })
    }
}

/// The unique identifier of a logical entity that can be an input or an output
/// of a build operation, as can be serialized to persistent storage.
///
/// See also [`RuntimeEntityIdent`], in which string values have been interned
/// into symbols.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
enum PersistEntityIdent {
    /// A source file. The string value is the path to the file in question,
    /// relative to the project root.
    SourceFile(String),

    /// An intermediate file. The string value is the path to the file in
    /// question, relative to the cache root.
    IntermediateFile(String),
}

impl PersistEntityIdent {
    fn compute_digest(&self) -> DigestData {
        let mut dc = DigestComputer::new();
        let data = bincode::serialize(self).unwrap(); // can this ever realistically fail?
        dc.update(data);
        dc.finalize()
    }
}

/// An "instance" of a build entity: a tuple of its identity and the digest of
/// its value, as can be serialized to persistent storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct PersistEntityInstance {
    /// The identity of the entity associated with this instance.
    pub ident: PersistEntityIdent,

    /// The digest of the entity associated with this instance.
    pub digest: DigestData,
}

/// The unique identifier of a logical entity that can be an input or an output
/// of a build operation, as managed during runtime. String values are interned
/// into symbols.
///
/// See also [`PersistEntityIdent`], in which interned string symbols have been
/// expanded into owned strings. That type can be serialized and deserialized,
/// whereas this type implements [`std::marker::Copy`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeEntityIdent {
    /// A source file. The symbol resolves to the path to the file in question,
    /// relative to the project root.
    SourceFile(PathSymbol),

    /// An intermediate file. The symbol resolves to the path to the file in
    /// question, relative to the cache root.
    IntermediateFile(PathSymbol),
}

/// An "instance" of a build entity: a tuple of its identity and the digest of
/// its value, as managed during runtime.
#[derive(Clone, Debug)]
pub struct RuntimeEntityInstance {
    /// The identity of the entity associated with this instance.
    pub ident: RuntimeEntityIdent,

    /// The digest of the entity associated with this instance.
    pub digest: DigestData,
}

/// The unique identifier for a build operation that we might wish to execute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpIdent {
    /// A "pass 1" TeX build.
    ///
    /// The symbol resolves to the relative path of its main input file.
    Pass1Build(PathSymbol),

    /// A "pass 2" TeX build.
    ///
    /// The symbol resolves to the relative path of its main input file.
    Pass2Build(PathSymbol),
}

/// An ordered set of instances, keyed and sorted by their persistent
/// identifiers.
///
/// These sets can be used to store and compare lists of inputs, for instance.
/// By ordering by the persistent identifiers, different executions of the code
/// will calculate the same overall digest even if the runtime load patterns
/// differ.
#[derive(Clone, Debug, Default)]
pub struct SortedPersistInstanceSet(BTreeMap<PersistEntityIdent, DigestData>);

impl SortedPersistInstanceSet {
    /// Insert a new entry into the set, based on its runtime identifier.
    ///
    /// If the identifier was already in the set, the previously associated
    /// digest is returned.
    pub fn insert_by_runtime_instance(
        &mut self,
        inst: RuntimeEntityInstance,
        cache: &Cache,
    ) -> Option<DigestData> {
        let pei = cache.persist_ident(inst.ident);
        self.0.insert(pei, inst.digest)
    }

    /// Insert a new entry into the set.
    ///
    /// If the identifier was already in the set, the previously associated
    /// digest is returned.
    fn insert_by_instance(&mut self, inst: PersistEntityInstance) -> Option<DigestData> {
        self.0.insert(inst.ident, inst.digest)
    }

    /// Compute the digest of this set of inputs.
    ///
    /// This digest is computed as the digest of all of the input digests,
    /// ordered by the persistent input identifiers. Thanks to this ordering,
    /// the returned value should be reproducible regardless of the order in
    /// which the inputs were added, so long as the inputs are the same.
    ///
    /// This does *not* include the input identies in the digest calculation. So
    /// the digest of a set containing one source file will be the same as the
    /// digest of a set containing a different file with identical contents.
    pub fn compute_digest(&self) -> DigestData {
        let mut dc = DigestComputer::new();

        for digest in self.0.values() {
            dc.update(digest);
        }

        dc.finalize()
    }

    /// Adapt this set into an iterator of instances.
    ///
    /// This clones each record in the set.
    fn as_instances(&self) -> impl Iterator<Item = PersistEntityInstance> + '_ {
        self.0.iter().map(|(k, v)| PersistEntityInstance {
            ident: k.clone(),
            digest: v.clone(),
        })
    }
}

/// An incremental build cache manager.
#[derive(Debug)]
pub struct Cache {
    /// The root directory of the cache filesystem tree.
    root: PathBuf,

    /// An interner for converting file path strings into symbols.
    paths: StringInterner,

    /// A table of saved digest information about input files. We only populate
    /// this map out of `loaded_file_digests` as we actually reference files, so
    /// that if files are removed we don't persist them indefinitely. We could
    /// also accomplish this with a "used" flag but I don't want to add an extra
    /// field to FileDigestEntry.
    file_digests: HashMap<RuntimeEntityIdent, FileDigestEntry>,

    /// File digests that we have loaded from disk at startup. Items will be
    /// removed from this map and moved into the main map as they are
    /// referenced.
    loaded_file_digests: HashMap<RuntimeEntityIdent, FileDigestEntry>,
}

impl Cache {
    /// Create a new incremental build cache manager.
    ///
    /// If there are errors loading the cache information, they will be printed
    /// to the status backend, but the cache will proceed as if the relevant
    /// cached information is simply missing. Hopefully this keep things robust
    /// if anything funny happens, although the user can always blow away the
    /// entire cache instead.
    pub fn new(status: &mut dyn StatusBackend) -> Result<Self> {
        let mut root = config::get_root()?;
        root.push("cache");

        let mut cache = Cache {
            root,
            file_digests: HashMap::new(),
            loaded_file_digests: HashMap::new(),
            paths: StringInterner::default(),
        };

        // Now we can (try to) load up the cache of file digest info.
        //
        // This might not scale very well ... but maybe it will be fine? Right
        // now I think it'll balance easy implementation and not being too
        // wasteful.

        let persisted_files: Vec<(PersistEntityIdent, FileDigestEntry)> = {
            let mut p_files = cache.root.clone();
            p_files.push("file_digests.dat");

            match fs::File::open(&p_files) {
                Ok(f) => match bincode::deserialize_from(f) {
                    Ok(pf) => pf,

                    Err(e) => {
                        tt_warning!(status, "error deserializing file data in `{}`", p_files.display(); e.into());
                        Vec::new()
                    }
                },

                Err(ref e) if e.kind() == ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e).context(format!("failed to open `{}`", p_files.display())),
            }
        };

        for pf in persisted_files {
            let pei = cache.depersist_ident(pf.0);
            cache.loaded_file_digests.insert(pei, pf.1);
        }

        // All done!

        Ok(cache)
    }

    fn intern_path(&mut self, p: impl AsRef<str>) -> PathSymbol {
        self.paths.get_or_intern(p)
    }

    /// Convert a [`PersistEntityIdent`] to a [`RuntimeEntityIdent`].
    ///
    /// This needs to go through the [`Cache`] type in order to intern
    /// the relevant strings.
    fn depersist_ident(&mut self, pei: PersistEntityIdent) -> RuntimeEntityIdent {
        match pei {
            PersistEntityIdent::SourceFile(p) => {
                RuntimeEntityIdent::SourceFile(self.intern_path(p))
            }

            PersistEntityIdent::IntermediateFile(p) => {
                RuntimeEntityIdent::IntermediateFile(self.intern_path(p))
            }
        }
    }

    /// Convert a [`RuntimeEntityIdent`] to a [`PersistEntityIdent`].
    ///
    /// This needs to go through the [`Cache`] type in order to resolve the
    /// interned strings.
    fn persist_ident(&self, rei: RuntimeEntityIdent) -> PersistEntityIdent {
        match rei {
            RuntimeEntityIdent::SourceFile(s) => {
                PersistEntityIdent::SourceFile(self.paths.resolve(s).unwrap().to_owned())
            }

            RuntimeEntityIdent::IntermediateFile(s) => {
                PersistEntityIdent::IntermediateFile(self.paths.resolve(s).unwrap().to_owned())
            }
        }
    }

    /// Convert a [`RuntimeEntityInstance`] to a [`PersistEntityInstance`].
    ///
    /// This needs to go through the [`Cache`] type in order to resolve the
    /// interned strings in the identifiers.
    fn persist_instance(&self, rei: RuntimeEntityInstance) -> PersistEntityInstance {
        PersistEntityInstance {
            ident: self.persist_ident(rei.ident),
            digest: rei.digest,
        }
    }

    /// Get a filesystem path associated with an entity in its persisted form,
    /// if one exists.
    ///
    /// This needs to go through the [`Cache`] type in order to know the path to
    /// the cache root.
    fn persist_entity_path(&self, o: &PersistEntityIdent) -> Option<PathBuf> {
        match o {
            PersistEntityIdent::SourceFile(relpath) => Some(relpath.to_owned().into()),

            PersistEntityIdent::IntermediateFile(relpath) => {
                let mut p = self.root.clone();
                p.push(relpath);
                Some(p)
            }
        }
    }

    /// Get a filesystem path associated with an entity in its runtime form, if
    /// one exists.
    ///
    /// This needs to go through the [`Cache`] type in order to know the path to
    /// the cache root and to resolve any interned strings.
    fn runtime_entity_path(&self, o: &RuntimeEntityIdent) -> Option<PathBuf> {
        match o {
            RuntimeEntityIdent::SourceFile(relpath) => {
                let p = self.paths.resolve(*relpath).unwrap();
                Some(p.to_owned().into())
            }

            RuntimeEntityIdent::IntermediateFile(relpath) => {
                let mut p = self.root.clone();
                p.push(self.paths.resolve(*relpath).unwrap());
                Some(p)
            }
        }
    }

    /// Get a [`RuntimeEntityInstance`] from a [`RuntimeEntityIdent`], if it
    /// resolves to a local filesystem path.
    ///
    /// This uses the file digest cache to hopefully load up the digest without
    /// actually rereading the file. But if the file isn't in the cache or if it
    /// seems to have been updated, we'll need to read it, so this function may
    /// have to do I/O. If this identifier does not actually correspond to a
    /// filesystem entity, the function panics.
    fn get_file_instance(&mut self, ident: RuntimeEntityIdent) -> Result<RuntimeEntityInstance> {
        if let Some(fentry) = self.file_digests.get(&ident) {
            return Ok(RuntimeEntityInstance {
                ident,
                digest: fentry.digest.clone(),
            });
        }

        // It's not in the active cache. Maybe it was on disk?
        let p = self
            .runtime_entity_path(&ident)
            .expect("internal get_file_instance called with non-file input?");

        let fentry = match self.loaded_file_digests.remove(&ident) {
            Some(mut fentry) => {
                atry!(
                    fentry.freshen(&p);
                    ["failed to probe input source file `{}`", p.display()]
                );
                fentry
            }

            // Nope, we need to start from scratch.
            None => {
                atry!(
                    FileDigestEntry::create(&p);
                    ["failed to probe input source file `{}`", p.display()]
                )
            }
        };

        self.file_digests.insert(ident.clone(), fentry.clone());

        Ok(RuntimeEntityInstance {
            ident,
            digest: fentry.digest.clone(),
        })
    }

    /// Get a "runtime entity instance" for a source file based on its path. We
    /// use the file digest cache to hopefully load up its digest without
    /// actually rereading the file. But if the file isn't in the cache or if it
    /// seems to have been updated, we'll need to read it, so this function may
    /// have to do I/O.
    fn get_source_file_instance(&mut self, p: impl AsRef<str>) -> Result<RuntimeEntityInstance> {
        let p = p.as_ref();
        let ident = RuntimeEntityIdent::SourceFile(self.intern_path(p));
        self.get_file_instance(ident)
    }

    /// Like [`Self::get_source_file_instance`], but always read the file in
    /// question to calculate its digest.
    fn make_source_file_instance(&mut self, p: impl AsRef<str>) -> Result<RuntimeEntityInstance> {
        let p = p.as_ref();
        let ident = RuntimeEntityIdent::SourceFile(self.intern_path(p));

        let fentry = atry!(
            FileDigestEntry::create(p);
            ["failed to probe input source file `{}`", p]
        );

        self.file_digests.insert(ident.clone(), fentry.clone());
        return Ok(RuntimeEntityInstance {
            ident,
            digest: fentry.digest.clone(),
        });
    }

    /// Generate a path within the cache tree based on a digest, optionally
    /// creating its containing directory.
    ///
    /// This operation is only fallible if *create* is true.
    fn cache_path(&self, dd: &DigestData, ext: &str, create: bool) -> Result<PathBuf> {
        let mut p = self.root.clone();
        let dd_hex = format!("{dd:x}.{ext}");
        p.push(&dd_hex[..2]);

        atry!(
            fs::create_dir_all(&p);
            ["failed to create directory tree `{}`", p.display()]
        );

        p.push(&dd_hex[2..]);
        Ok(p)
    }

    /// Load a vector of instances from a file identified by a digest.
    ///
    /// This is used to load "extra inputs" associated with a given TeX file
    /// after we have run a build and learned what external files it depends on.
    fn load_instances(&mut self, dd: &DigestData) -> Result<Vec<PersistEntityInstance>> {
        let p = self.cache_path(dd, "insts", false).unwrap();

        let f = match fs::File::open(&p) {
            Ok(f) => f,
            Err(ref e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).context(format!("failed to open `{}`", p.display())),
        };

        Ok(atry!(
            bincode::deserialize_from(f);
            ["failed to deserialize bincode data from `{}`", p.display()]
        ))
    }

    /// Load a vector of instances into a file identified by a digest.
    ///
    /// This is used to save "extra inputs" associated with a given TeX file
    /// after we have run a build and learned what external files it depends on.
    /// The saving is done with a temporary file and an atomic rename.
    fn save_instances(
        &mut self,
        dd: &DigestData,
        insts: impl IntoIterator<Item = PersistEntityInstance>,
    ) -> Result<()> {
        let v: Vec<PersistEntityInstance> = insts.into_iter().collect();

        let p = self.cache_path(dd, "insts", true)?;
        let dir = p.parent().unwrap();

        let mut f = atry!(
            NamedTempFile::new_in(dir);
            ["failed to create temporary file in `{}`", dir.display()]
        );

        atry!(
            bincode::serialize_into(&mut f, &v);
            ["failed to serialize bincode data into `{}`", f.path().display()]
        );

        atry!(
            f.persist(&p);
            ["failed to persist temporary file into `{}`", p.display()]
        );

        Ok(())
    }

    /// Turn an [`OpIdent`] into a digest.
    ///
    /// This needs the cache to resolve interned symbols associated with the
    /// ident.
    fn opid_digest(&self, opid: &OpIdent) -> DigestData {
        let mut dc = DigestComputer::new();

        match opid {
            OpIdent::Pass1Build(inp) => {
                // Bump the version number if the nature of the pass-1 operation
                // changes such that we should discard all cached results.
                dc.update("pass1_v1");
                dc.update(self.paths.resolve(*inp).unwrap());
            }

            OpIdent::Pass2Build(inp) => {
                dc.update("pass2_v1");
                dc.update(self.paths.resolve(*inp).unwrap());
            }
        }

        dc.finalize()
    }

    /// Test whether the output of a build operation exists.
    fn output_exists(
        &self,
        o: &PersistEntityIdent,
        p_cache: &Path,
        status: &mut dyn StatusBackend,
    ) -> Result<bool> {
        if let PersistEntityIdent::SourceFile(p) = o {
            tt_warning!(
                status,
                "illegal source-file output `{}` declared in `{}`",
                p,
                p_cache.display()
            );

            // Effectively ignore this record by saying that it exists.
            return Ok(true);
        }

        if let Some(p) = self.persist_entity_path(o) {
            match fs::metadata(&p) {
                Ok(_) => Ok(true),
                Err(ref e) if e.kind() == ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e).context(format!("failed to probe path `{}`", p.display())),
            }
        } else {
            // Not a filesystem thing; for our purposes, that means it exists.
            Ok(true)
        }
    }

    /// Make an [`OpCacheData`] for a TeX pass-1 operation.
    ///
    /// This generates the op-id and sets the "source input" field.
    pub fn make_pass1_cache_data(&mut self, input_relpath: impl AsRef<str>) -> Result<OpCacheData> {
        let input_relpath = input_relpath.as_ref();
        let opid = OpIdent::Pass1Build(self.intern_path(input_relpath));
        let mut data = OpCacheData::new(opid);
        let src_input = self.make_source_file_instance(input_relpath)?;
        data.set_src_input(src_input);
        Ok(data)
    }

    /// Returns true if the specified operation needs to be rerun.
    ///
    /// The input cache data should have been set up information about the
    /// expected inputs to the "runtime" form of the operation.
    ///
    /// The build can be skipped if implementation of the operation is unchanged
    /// (which will almost always be the case); the inputs to the operation are
    /// unchanged; and the outputs of the operation all still exist. We don't
    /// test whether the *outputs* are unchanged from what we thought they were,
    /// both for efficiency (if our implementation is correct, we don't need to)
    /// and because that way we let the user manually edit outputs and rebuild
    /// if needed.
    ///
    /// I/O errors relating to the cache data will be reported to the status
    /// backend but not exposed up the call chain, to try to keep things robust
    /// even if something funny happens in the cache.
    pub fn operation_needs_rerun(
        &mut self,
        data: &OpCacheData,
        status: &mut dyn StatusBackend,
    ) -> Result<bool> {
        // If the cache record for this operation doesn't exist, we must rerun
        // the operation.

        let p_cache = self
            .cache_path(&self.opid_digest(&data.opid), "op", false)
            .unwrap();

        let mut f_cache = match fs::File::open(&p_cache) {
            Ok(f) => f,
            Err(e) => {
                if e.kind() != ErrorKind::NotFound {
                    tt_warning!(
                        status,
                        "error reading build cache file `{}`", p_cache.display();
                        e.into()
                    );
                }

                return Ok(true);
            }
        };

        // Gather inputs: the input file, and any extra deps that we may have
        // detected in previous executions.

        let mut inputs = SortedPersistInstanceSet::default();

        if let Some(src_input) = data.src_input.as_ref() {
            inputs.insert_by_runtime_instance(src_input.clone(), self);

            let eid = self.persist_ident(src_input.ident).compute_digest();
            let mut extra_inputs = atry!(
                self.load_instances(&eid);
                ["error loading extra input data for `{src_input:?}`"]
            );

            for pei in extra_inputs.drain(..) {
                inputs.insert_by_instance(pei);
            }
        }

        // TODO: additional non-"extra" inputs! (?)

        let actual_input_digest = inputs.compute_digest();

        // If the inputs have changed, we must rerun the operation.

        let saved_input_digest = match bincode::deserialize_from(&mut f_cache) {
            Ok(d) => d,
            Err(e) => {
                tt_warning!(status, "failed to deserialize bincode data from `{}` (1)", p_cache.display(); e.into());
                return Ok(true);
            }
        };

        if actual_input_digest != saved_input_digest {
            return Ok(true);
        }

        // If we're still here, gather information about the outputs and check
        // them. If any don't exist, we need to rerun.

        let saved_outputs: Vec<PersistEntityIdent> = match bincode::deserialize_from(&mut f_cache) {
            Ok(d) => d,
            Err(e) => {
                tt_warning!(status, "failed to deserialize bincode data from `{}` (2)", p_cache.display(); e.into());
                return Ok(true);
            }
        };

        for o in &saved_outputs {
            if !self.output_exists(o, &p_cache, status)? {
                return Ok(true);
            }
        }

        // If we haven't spotted any problems, we don't need to rerun this step!

        Ok(false)
    }

    /// Mark an operation as complete and cache its information so that we can
    /// know whether it needs to be rerun in the future.
    ///
    /// The cache file is created using a temporary file and an atomic rename so
    /// that partially-complete files are not observed.
    pub fn finalize_operation(&mut self, mut data: OpCacheData) -> Result<()> {
        // Start creating the cache file.

        let p_cache = self.cache_path(&self.opid_digest(&data.opid), "op", true)?;

        let mut f_cache = atry!(
            NamedTempFile::new_in(&self.root);
            ["failed to create temporary file `{}`", self.root.display()]
        );

        // If a "source input" is defined, save the extra inputs for future
        // steps involving the same input.

        if let Some(src_input) = data.src_input {
            let eid = self.persist_ident(src_input.ident).compute_digest();
            atry!(
                self.save_instances(&eid, data.extra_inputs.as_instances());
                ["error saving extra input data for `{src_input:?}`"]
            );

            // Now add this to the extra_inputs, which we're about to use for
            // computing the total input hash.
            data.extra_inputs
                .insert_by_runtime_instance(src_input, self);
        }

        // Compute the total hash of all of the inputs and save it in the cache file.

        let inputs_digest = data.extra_inputs.compute_digest();

        atry!(
            bincode::serialize_into(&mut f_cache, &inputs_digest);
            ["failed to serialize bincode data into `{}`", p_cache.display()]
        );

        // Log the output identities. We don't persist digests since those aren't
        // relevant to figuring out whether this step needs rerunning; we just need
        // to verify that the outputs exist.

        let pids: Vec<PersistEntityIdent> = data
            .outputs
            .iter()
            .map(|rei| self.persist_ident(*rei))
            .collect();

        atry!(
            bincode::serialize_into(&mut f_cache, &pids);
            ["failed to serialize bincode data into `{}`", p_cache.display()]
        );

        // And that's it!

        atry!(
            f_cache.persist(&p_cache);
            ["failed to persist temporary cache file to `{}`", p_cache.display()]
        );
        Ok(())
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
    /// Create a new intermediate output file.
    ///
    /// The file's path is relative to the `cache/` subdirectory. Parent
    /// directories will be created as needed.
    pub fn new_intermediate(relpath: impl AsRef<str>, cache: &mut Cache) -> Result<Self> {
        let relpath = relpath.as_ref();
        let ident = RuntimeEntityIdent::IntermediateFile(cache.intern_path(relpath));
        let path = cache.runtime_entity_path(&ident).unwrap();

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
                ["failed to create temporary file"]
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

    /// Close the stream and return an entity instance corresponding to its
    /// contents.
    ///
    /// This consumes the object.
    ///
    /// This operation uses standard Rust drop semantics to close the output
    /// file, and so cannot detect any I/O errors that occur as the file is
    /// closed.
    pub fn close(mut self, cache: &mut Cache) -> Result<RuntimeEntityInstance> {
        atry!(
            self.flush();
            ["failed to flush file `{}`", self.path.display()]
        );

        let path = self.path;

        atry!(
            self.file.persist(&path);
            ["failed to persist temporary file to `{}`", path.display()]
        );

        let digest = self.dc.finalize();

        // Make sure we have a nice fresh cache record for this new file, whose
        // digest we've just bothered to compute.
        let entry = atry!(
            FileDigestEntry::create_for_known(&path, digest.clone(), self.size);
            ["failed to probe metadata for file `{}`", path.display()]
        );
        cache.file_digests.insert(self.ident.clone(), entry);

        Ok(RuntimeEntityInstance {
            ident: self.ident,
            digest,
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

#[derive(Debug)]
pub struct OpCacheData {
    opid: OpIdent,
    src_input: Option<RuntimeEntityInstance>,
    extra_inputs: SortedPersistInstanceSet,
    outputs: Vec<RuntimeEntityIdent>,
}

impl OpCacheData {
    /// Create a new cacher for this operation.
    pub fn new(opid: OpIdent) -> Self {
        OpCacheData {
            opid,
            src_input: None,
            extra_inputs: Default::default(),
            outputs: Default::default(),
        }
    }

    /// Set the "source input" for this operation.
    ///
    /// If specified, any "extra inputs" added with
    /// [`Self::add_extra_input_source_file`] will be logged as associated with
    /// this particular input. Future runs of this build operation, and any
    /// other build operations *also* associated with this source input, will be
    /// rerun if any of these files change. This allows us to dynamically add
    /// build dependencies discovered while evaluating input files (e.g., a TeX
    /// file depends on a PNG file; we can't know that in advance, but once we
    /// have done a build, if the PNG file changes, we should rebuild
    /// appropriately).
    pub fn set_src_input(&mut self, inst: RuntimeEntityInstance) -> &mut Self {
        self.src_input = Some(inst);
        self
    }

    /// Register an "extra input" to be associated with this operation's "source
    /// input".
    ///
    /// See [`Self::set_src_input`].
    pub fn add_extra_input_source_file(
        &mut self,
        relpath: impl AsRef<str>,
        cache: &mut Cache,
    ) -> Result<&mut Self> {
        let inst = cache.get_source_file_instance(relpath.as_ref())?;
        self.extra_inputs.insert_by_runtime_instance(inst, cache);
        Ok(self)
    }

    /// Register an output that is associated with this operation.
    ///
    /// Note that only the output identity is needed, not its full instance.
    pub fn add_output(&mut self, ident: RuntimeEntityIdent) -> &mut Self {
        self.outputs.push(ident);
        self
    }
}
