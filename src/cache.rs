// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Caching infrastructure for incremental builds.
//!
//! We don't try to implement a full dependency graph structure since our build
//! process is fairly simple. But, the infrastructure for checking whether
//! specific build operations need to be rerun aims to be flexible so that it
//! can capture lots of different steps.

use bincode;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_warning, StatusBackend};
use tempfile::NamedTempFile;

use crate::{
    config,
    index::IndexCollection,
    operation::{
        DigestComputer, DigestData, PersistEntity, PersistEntityIdent, RuntimeEntity,
        RuntimeEntityIdent,
    },
};

/// Helper for caching file digests based on modification times.
///
/// Most of our build inputs and outputs are files on disk. It would get pretty
/// slow to recalculate the digest of their contents every time we needed to
/// check if they've changed. So, we do what virtually every other build system
/// does, and we cache file digests based on their filesystem metadata,
/// specifically modification times and sizes. If a file's mtime and size are
/// what we have in our cache, we assume that its digest is as well.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct FileDigestEntry {
    digest: DigestData,
    mtime: SystemTime,
    size: u64,
}

/// Calculate the digest and size of a file, reading the whole thing.
fn digest_of_file(p: impl AsRef<Path>) -> io::Result<(u64, DigestData)> {
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
    fn create(p: impl AsRef<Path>) -> io::Result<FileDigestEntry> {
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
    ///
    /// We return a [`std::io::Result`] so that callers can easily test if the
    /// file in question did not exist.
    fn freshen(&mut self, p: impl AsRef<Path>) -> io::Result<()> {
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
    fn create_for_known(
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

/// An incremental build cache manager.
#[derive(Debug)]
pub struct Cache {
    /// The root directory of the cache filesystem tree.
    root: PathBuf,

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
    pub fn new(indices: &mut IndexCollection, status: &mut dyn StatusBackend) -> Result<Self> {
        let root = config::get_root()?;

        let mut cache = Cache {
            root,
            file_digests: HashMap::new(),
            loaded_file_digests: HashMap::new(),
        };

        // Now we can (try to) load up the cache of file digest info.
        //
        // This might not scale very well ... but maybe it will be fine? Right
        // now I think it'll balance easy implementation and not being too
        // wasteful.

        let persisted_files: Vec<(PersistEntityIdent, FileDigestEntry)> = {
            let mut p_files = cache.root.clone();
            p_files.push("cache");
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
            let pei = indices.runtime_ident(&pf.0);
            cache.loaded_file_digests.insert(pei, pf.1);
        }

        // All done!

        Ok(cache)
    }

    /// Generate a path within the cache tree based on a digest, optionally
    /// creating its containing directory.
    ///
    /// This operation is only fallible if *create* is true.
    fn cache_path(&self, dd: &DigestData, ext: &str, create: bool) -> Result<PathBuf> {
        let mut p = self.root.clone();
        p.push("cache");

        let dd_hex = format!("{dd:x}.{ext}");
        p.push(&dd_hex[..2]);

        if create {
            atry!(
                fs::create_dir_all(&p);
                ["failed to create directory tree `{}`", p.display()]
            );
        }

        p.push(&dd_hex[2..]);
        Ok(p)
    }

    /// Get a [`RuntimeEntity`] from a [`RuntimeEntityIdent`], if it
    /// resolves to a local filesystem path.
    ///
    /// This uses the file digest cache to hopefully load up the digest without
    /// actually rereading the file. But if the file isn't in the cache or if it
    /// seems to have been updated, we'll need to read it, so this function may
    /// have to do I/O. If this identifier does not actually correspond to a
    /// filesystem entity, the function panics.
    fn get_file_entity(
        &mut self,
        ident: RuntimeEntityIdent,
        indices: &IndexCollection,
    ) -> Result<Option<RuntimeEntity>> {
        if let Some(fentry) = self.file_digests.get(&ident) {
            return Ok(Some(RuntimeEntity {
                ident,
                value_digest: fentry.digest.clone(),
            }));
        }

        // It's not in the active cache. Maybe it was on disk?
        //
        // We can just unwrap this result since right now the only idents that
        // exist are ones associated with paths.
        let p = indices.path_for_runtime_ident(ident).unwrap();

        let fentry = match self.loaded_file_digests.remove(&ident) {
            // Yes. If the filesystem metadata match our cache, let's say
            // that we're good.
            Some(mut fentry) => fentry.freshen(&p).map(|_| fentry),

            // Nope, we need to start from scratch.
            None => FileDigestEntry::create(&p),
        };

        // If the file didn't exist, handle that specially -- that's not a give-up error.
        let fentry = match fentry {
            Ok(f) => f,
            Err(ref e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(format!("failed to probe file `{}`", p.display())),
        };

        self.file_digests.insert(ident.clone(), fentry.clone());

        Ok(Some(RuntimeEntity {
            ident,
            value_digest: fentry.digest.clone(),
        }))
    }

    /// Make a [`RuntimeEntity`] from a [`RuntimeEntityIdent`], ensuring that we
    /// have an up-to-date digest of its value.
    ///
    /// If the return value is `Ok(None)`, the entity doesn't exist in the
    /// environment, and processing should presumably rebuild whatever depends
    /// on the entity. If the return value is `Err`, processing should give up
    /// -- some kind of unexpected error happened.
    fn read_entity(
        &mut self,
        ident: RuntimeEntityIdent,
        indices: &IndexCollection,
    ) -> Result<Option<RuntimeEntity>> {
        // One day we may have different entity classes, but right now, we only
        // have files.
        self.get_file_entity(ident, indices)
    }

    /// Make a [`RuntimeEntity`] from a [`RuntimeEntityIdent`], ensuring that we
    /// have an up-to-date digest of its value.
    pub fn require_entity(
        &mut self,
        ident: RuntimeEntityIdent,
        indices: &IndexCollection,
    ) -> Result<RuntimeEntity> {
        match self.read_entity(ident, indices) {
            Ok(Some(e)) => Ok(e),
            Ok(None) => {
                // Kind of janky; should maybe refactor
                let p = indices.path_for_runtime_ident(ident).unwrap();
                bail!("failed to probe file `{}`: it does not exist", p.display());
            }
            Err(e) => Err(e),
        }
    }

    /// Make a [`RuntimeEntity`] from a [`RuntimeEntityIdent`], deriving an
    /// up-to-date digest of its value if possible. If the entity does not
    /// currently exist in the environment, a null digest value is used, so that
    /// any subsequent comparison with a real digest should indicate a change.
    /// If the return value is `Err`, some kind of unexpected error happened and
    /// processing should give up.
    pub fn unconditional_entity(
        &mut self,
        ident: RuntimeEntityIdent,
        indices: &IndexCollection,
    ) -> Result<RuntimeEntity> {
        self.read_entity(ident, indices).map(|e| {
            e.unwrap_or_else(|| {
                let value_digest = DigestData::default();
                RuntimeEntity {
                    ident,
                    value_digest,
                }
            })
        })
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
        opid: &DigestData,
        indices: &mut IndexCollection,
        status: &mut dyn StatusBackend,
    ) -> Result<bool> {
        // If the cache record for this operation doesn't exist, we must rerun
        // the operation.

        let p_cache = self.cache_path(opid, "op", false).unwrap();

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

        // Gather saved inputs. These should be in sorted order so that we can
        // regenerate the digest of the inputs in the same way that the sorted
        // set produces.

        let saved_inputs: Vec<PersistEntityIdent> = match bincode::deserialize_from(&mut f_cache) {
            Ok(d) => d,
            Err(e) => {
                tt_warning!(status, "failed to deserialize bincode data from `{}`", p_cache.display(); e.into());
                return Ok(true);
            }
        };

        let mut dc = DigestComputer::default();

        for pei in &saved_inputs {
            let rei = indices.runtime_ident(pei);

            match self.read_entity(rei, indices)? {
                Some(e) => {
                    dc.update(e.value_digest);
                }

                None => {
                    // The entity does not exist -- we need to rerun
                    return Ok(true);
                }
            }
        }

        let actual_input_digest = dc.finalize();

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
            if !o.artifact_exists(self.root.clone())? {
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
    pub fn finalize_operation(
        &mut self,
        mut data: OpCacheData,
        indices: &mut IndexCollection,
    ) -> Result<()> {
        // Start creating the cache file.

        let p_cache = self.cache_path(&data.ident, "op", true)?;

        let mut f_cache = atry!(
            NamedTempFile::new_in(&self.root);
            ["failed to create temporary file `{}`", self.root.display()]
        );

        // Get digests for all of the inputs.

        let mut input_set = SortedPersistEntitySet::default();

        for rei in data.inputs.drain(..) {
            let ent = self.require_entity(rei, indices)?;
            input_set.insert_runtime(ent, indices);
        }

        // Compute their total hash and save everything.

        let mut input_idents: Vec<PersistEntityIdent> = Vec::with_capacity(data.inputs.len());
        let mut dc = DigestComputer::default();

        // BTreeMap doesn't have a stable drain API, so this is not as efficient as
        // it could be.
        for input in input_set.as_entities() {
            input_idents.push(input.ident);
            dc.update(input.value_digest);
        }

        atry!(
            bincode::serialize_into(&mut f_cache, &input_idents);
            ["failed to serialize bincode data into `{}`", p_cache.display()]
        );

        let inputs_digest = dc.finalize();

        atry!(
            bincode::serialize_into(&mut f_cache, &inputs_digest);
            ["failed to serialize bincode data into `{}`", p_cache.display()]
        );

        // Log the output identities. We don't persist digests, or worry about
        // ordering, since those aren't relevant to figuring out whether this
        // step needs rerunning; we just need to verify that the outputs exist.

        let mut pids: Vec<PersistEntityIdent> = Vec::with_capacity(data.outputs.len());

        for info in data.outputs.drain(..) {
            pids.push(indices.persist_ident(info.0));

            // An output file may have been modified. If we know what the new
            // digest is, we can update our cache immediately. If not, make sure
            // to remove it from the cache, so that we will recompute its digest
            // if it is requested later.
            if let Some((value_digest, size)) = info.1 {
                // Potentially risky to unwrap here
                let path = indices.path_for_runtime_ident(info.0).unwrap();
                let fentry = FileDigestEntry::create_for_known(&path, value_digest, size)?;
                self.file_digests.insert(info.0, fentry);
            } else {
                self.file_digests.remove(&info.0);
            }
        }

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

#[derive(Debug)]
pub struct OpCacheData {
    ident: DigestData,
    inputs: Vec<RuntimeEntityIdent>,
    outputs: Vec<(RuntimeEntityIdent, Option<(DigestData, u64)>)>,
}

impl OpCacheData {
    /// Create a new operation caching data structure.
    pub fn new(ident: DigestData) -> Self {
        OpCacheData {
            ident,
            inputs: Default::default(),
            outputs: Default::default(),
        }
    }

    /// Log an input to this operation.
    pub fn add_input(&mut self, ident: RuntimeEntityIdent) -> &mut Self {
        self.inputs.push(ident);
        self
    }

    /// Register an output that is associated with this operation.
    ///
    /// Note that only the output identity is needed, not its full instance.
    pub fn add_output(&mut self, ident: RuntimeEntityIdent) -> &mut Self {
        self.outputs.push((ident, None));
        self
    }

    /// Register an output that is associated with this operation, plus
    /// information about its value.
    ///
    /// While the value information is not needed for caching the results of
    /// this operation, we can use it to be more efficient if this output is
    /// used as the input of another operation.
    pub fn add_output_with_value(
        &mut self,
        ident: RuntimeEntityIdent,
        value_digest: DigestData,
        size: u64,
    ) -> &mut Self {
        self.outputs.push((ident, Some((value_digest, size))));
        self
    }
}

/// An ordered set of entities, keyed and sorted by their persistent
/// identifiers.
///
/// These sets can be used to store and compare lists of inputs, for instance.
/// By ordering by the persistent identifiers, different executions of the code
/// will calculate the same overall digest even if the runtime load patterns
/// differ.
#[derive(Clone, Debug, Default)]
pub struct SortedPersistEntitySet(BTreeMap<PersistEntityIdent, DigestData>);

impl SortedPersistEntitySet {
    /// Insert a new entity into the set, based on its runtime identifier.
    ///
    /// If the entity was already in the set, the previously associated
    /// digest is returned.
    pub fn insert_runtime(
        &mut self,
        re: RuntimeEntity,
        indices: &IndexCollection,
    ) -> Option<DigestData> {
        let pi = indices.persist_ident(re.ident);
        self.0.insert(pi, re.value_digest)
    }

    /// Adapt this set into an iterator of entities.
    ///
    /// This clones each record in the set, and produces items in their sort
    /// order.
    fn as_entities(&self) -> impl Iterator<Item = PersistEntity> + '_ {
        self.0.iter().map(|(k, v)| PersistEntity {
            ident: k.clone(),
            value_digest: v.clone(),
        })
    }
}
