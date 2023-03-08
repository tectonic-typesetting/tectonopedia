// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Caching infrastructure for incremental builds.
//!
//! We don't try to implement a full dependency graph structure since our build
//! process is fairly simple. But, the infrastructure for checking whether
//! specific build operations need to be rerun aims to be flexible so that it
//! can capture lots of different steps.

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

use crate::config;

/// A type t
pub type DigestComputer = Sha256;
pub type DigestData = GenericArray<u8, <DigestComputer as OutputSizeUser>::OutputSize>;

type PathSymbol = DefaultSymbol;

// Helper for caching file digests based on mtimes

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct FileDigestEntry {
    pub digest: DigestData,
    mtime: SystemTime,
    size: u64,
}

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

    /// Make sure that the information associated with this instance is fresh.
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
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
enum PersistEntityIdent {
    /// A source file. The string value is the path to the file in question,
    /// relative to the project root.
    SourceFile(String),

    /// An intermediate file. The string value is the path to the file in
    /// question, relative to the cache root.
    IntermediateFile(String),
}

/// An "instance" of a build entity: a tuple of its identity and the digest of
/// its value, as can be serialized to persistent storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct PersistEntityInstance {
    pub ident: PersistEntityIdent,
    pub digest: DigestData,
}

/// The unique identifier of a logical entity that can be an input or an output
/// of a build operation, as managed during runtime. String values are interned
/// into symbols.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeEntityIdent {
    SourceFile(PathSymbol),
    IntermediateFile(PathSymbol),
}

#[derive(Clone, Debug)]
pub struct RuntimeEntityInstance {
    pub ident: RuntimeEntityIdent,
    pub digest: DigestData,
}

/// The unique identifier for a build operation that we might wish to execute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationIdent {
    Pass1Build(PathSymbol),
    Pass2Build(PathSymbol),
}

trait Digestible {
    fn update_digest(&self, dc: &mut DigestComputer);

    fn compute_digest(&self) -> DigestData {
        let mut dc = DigestComputer::new();
        self.update_digest(&mut dc);
        dc.finalize()
    }
}

#[derive(Debug)]
pub struct Cache {
    root: PathBuf,
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

    fn persist_instance(&self, rei: RuntimeEntityInstance) -> PersistEntityInstance {
        PersistEntityInstance {
            ident: self.persist_ident(rei.ident),
            digest: rei.digest,
        }
    }

    /// Get a "runtime entity instance" for a source file based on its path. We
    /// use the file digest cache to hopefully load up its digest without
    /// actually rereading the file. But if the file isn't in the cache or if it
    /// seems to have been updated, we'll need to read it, so this function may
    /// have to do I/O.
    fn get_source_file_instance(&mut self, p: impl AsRef<str>) -> Result<RuntimeEntityInstance> {
        let p = p.as_ref();
        let ident = RuntimeEntityIdent::SourceFile(self.intern_path(p));

        if let Some(fentry) = self.file_digests.get(&ident) {
            return Ok(RuntimeEntityInstance {
                ident,
                digest: fentry.digest.clone(),
            });
        }

        // It's not in the active cache. Maybe it was on disk?

        let fentry = match self.loaded_file_digests.remove(&ident) {
            Some(mut fentry) => {
                atry!(
                    fentry.freshen(p);
                    ["failed to probe input source file `{}`", p]
                );
                fentry
            }

            // Nope, we need to start from scratch.
            None => {
                atry!(
                    FileDigestEntry::create(p);
                    ["failed to probe input source file `{}`", p]
                )
            }
        };

        self.file_digests.insert(ident.clone(), fentry.clone());
        return Ok(RuntimeEntityInstance {
            ident,
            digest: fentry.digest.clone(),
        });
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

    fn cache_path(&self, dd: &DigestData, ext: &str) -> PathBuf {
        let mut p = self.root.clone();
        let dd_hex = format!("{dd:x}.{ext}");
        p.push(&dd_hex[..2]);
        p.push(&dd_hex[2..]);
        p
    }

    fn load_instances(&mut self, dd: &DigestData) -> Result<Vec<PersistEntityInstance>> {
        let p = self.cache_path(dd, "insts");

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

    fn save_instances(
        &mut self,
        dd: &DigestData,
        insts: impl IntoIterator<Item = RuntimeEntityInstance>,
    ) -> Result<()> {
        let v: Vec<PersistEntityInstance> = insts
            .into_iter()
            .map(|r| self.persist_instance(r))
            .collect();

        let p = self.cache_path(dd, "insts");

        if let Some(dir) = p.parent() {
            atry!(
                fs::create_dir_all(dir);
                ["failed to create directory tree `{}`", dir.display()]
            );
        }

        let f = atry!(
            fs::File::create(&p);
            ["failed to create file `{}`", p.display()]
        );

        Ok(atry!(
            bincode::serialize_into(f, &v);
            ["failed to serialize bincode data into `{}`", p.display()]
        ))
    }

    fn opid_digest(&self, opid: &OperationIdent) -> DigestData {
        let mut dc = DigestComputer::new();

        match opid {
            OperationIdent::Pass1Build(inp) => {
                // Bump the version number if the nature of the pass-1 operation
                // changes such that we should discard all cached results.
                dc.update("pass1_v1");
                dc.update(self.paths.resolve(*inp).unwrap());
            }

            OperationIdent::Pass2Build(inp) => {
                dc.update("pass2_v1");
                dc.update(self.paths.resolve(*inp).unwrap());
            }
        }

        dc.finalize()
    }

    /// Get a filesystem path associated with an entity in its persisted form,
    /// if one exists.
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

    /// Get a filesystem path associated with an entity in its runtime form,
    /// if one exists.
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

    /// Returns true if the pass-1 build stage needs to be run for the given
    /// input file.
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
    pub fn input_needs_pass1(
        &mut self,
        input_relpath: &str,
        status: &mut dyn StatusBackend,
    ) -> Result<bool> {
        // Construct the operation identifier.

        let opid = OperationIdent::Pass1Build(self.intern_path(input_relpath));

        // If the cache record for this operation doesn't exist, we must rerun
        // the operation.

        let p_cache = self.cache_path(&self.opid_digest(&opid), "op");
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

        let mut inputs = BTreeMap::new();

        let inp_digest = self.get_source_file_instance(input_relpath)?;
        inputs.insert(inp_digest.ident, inp_digest.digest);

        let eid = extra_inputs_digest(input_relpath);
        let mut extra_inputs = atry!(
            self.load_instances(&eid);
            ["error loading extra input data for `{input_relpath}`"]
        );

        for pei in extra_inputs.drain(..) {
            inputs.insert(self.depersist_ident(pei.ident), pei.digest);
        }

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
}

fn extra_inputs_digest(input_relpath: &str) -> DigestData {
    let mut dc = DigestComputer::new();
    dc.update("extra_inputs");
    dc.update(input_relpath);
    dc.finalize()
}

impl Digestible for BTreeMap<RuntimeEntityIdent, DigestData> {
    fn update_digest(&self, dc: &mut DigestComputer) {
        for digest in self.values() {
            dc.update(digest);
        }
    }
}

/// A helper to dynamically convert one of our common BTreeMaps into an iterator
/// of "instances". We can't do this with an extension trait since I don't want
/// to to write this function without `impl Trait` in return position, which we
/// can't to in a trait.
fn runtime_btree_as_instances(
    btree: &BTreeMap<RuntimeEntityIdent, DigestData>,
) -> impl Iterator<Item = RuntimeEntityInstance> + '_ {
    btree.iter().map(|(k, v)| RuntimeEntityInstance {
        ident: k.clone(),
        digest: v.clone(),
    })
}

/// A helper for creating build output files that are streamed to disk.
///
/// This class calculates the cryptographic digest as the data are written, so
/// that it can be cached efficiently.
#[derive(Debug)]
pub struct OpOutputStream {
    ident: RuntimeEntityIdent,
    path: PathBuf,
    file: fs::File,
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

        if let Some(dir) = path.parent() {
            atry!(
                fs::create_dir_all(dir);
                ["failed to create directory tree `{}`", dir.display()]
            );
        }

        let file = atry!(
            fs::File::create(&path);
            ["failed to create file `{}`", path.display()]
        );

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

        std::mem::drop(self.file);

        let path = self.path;
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
pub struct Pass1Cacher {
    opid: OperationIdent,
    src_input: RuntimeEntityInstance,
    extra_inputs: BTreeMap<RuntimeEntityIdent, DigestData>,
    assets: OpOutputStream,
    metadata: OpOutputStream,
}

impl Pass1Cacher {
    pub fn new(input_relpath: impl AsRef<str>, cache: &mut Cache) -> Result<Self> {
        let input_relpath = input_relpath.as_ref();
        let opid = OperationIdent::Pass1Build(cache.intern_path(input_relpath));

        let src_input = cache.make_source_file_instance(input_relpath)?;
        let extra_inputs = BTreeMap::new();

        let stripped = input_relpath.strip_suffix(".tex").unwrap_or(input_relpath);
        let assets = OpOutputStream::new_intermediate(&format!("pass1/{stripped}.assets"), cache)?;
        let metadata = OpOutputStream::new_intermediate(&format!("pass1/{stripped}.meta"), cache)?;

        Ok(Pass1Cacher {
            opid,
            src_input,
            extra_inputs,
            assets,
            metadata,
        })
    }

    pub fn metadata_line(&mut self, line: impl AsRef<str>) -> Result<()> {
        Ok(atry!(
            writeln!(&mut self.metadata, "{}", line.as_ref());
            ["error writing to `{}`", self.metadata.display_path()]
        ))
    }

    pub fn assets_line(&mut self, line: impl AsRef<str>) -> Result<()> {
        Ok(atry!(
            writeln!(&mut self.assets, "{}", line.as_ref());
            ["error writing to `{}`", self.assets.display_path()]
        ))
    }

    pub fn add_extra_source_file_input(
        &mut self,
        relpath: impl AsRef<str>,
        cache: &mut Cache,
    ) -> Result<()> {
        let inst = cache.get_source_file_instance(relpath.as_ref())?;
        self.extra_inputs.insert(inst.ident, inst.digest);
        Ok(())
    }

    /// Mark the operation as complete and cache its information so that we can
    /// know whether it needs to be rerun in the future.
    pub fn finalize(self, cache: &mut Cache) -> Result<()> {
        // Close out the output files.

        let assets = self.assets.close(cache)?;
        let metadata = self.metadata.close(cache)?;

        // Create the cache file.

        let p_cache = cache.cache_path(&cache.opid_digest(&self.opid), "op");

        if let Some(dir) = p_cache.parent() {
            atry!(
                fs::create_dir_all(dir);
                ["failed to create directory tree `{}`", dir.display()]
            );
        }

        let mut f_cache = atry!(
            fs::File::create(&p_cache);
            ["failed to create build cache file `{}`", p_cache.display()]
        );

        // Save the extra inputs for future steps involving the same input TeX
        // file.

        let input_relpath = match self.src_input.ident {
            RuntimeEntityIdent::SourceFile(s) => cache.paths.resolve(s).unwrap().to_owned(),
            _ => unreachable!(),
        };

        let eid = extra_inputs_digest(&input_relpath);
        atry!(
            cache.save_instances(&eid, runtime_btree_as_instances(&self.extra_inputs));
            ["error saving extra input data for `{input_relpath}`"]
        );

        // Compute the total hash of all of the inputs and save it in the cache file.

        let mut inputs = self.extra_inputs;
        inputs.insert(self.src_input.ident, self.src_input.digest);
        let inputs_digest = inputs.compute_digest();

        atry!(
            bincode::serialize_into(&mut f_cache, &inputs_digest);
            ["failed to serialize bincode data into `{}`", p_cache.display()]
        );

        // Log the output identities. We don't persist digests since those aren't
        // relevant to figuring out whether this step needs rerunning; we just need
        // to verify that the outputs exist.

        let mut outputs: Vec<PersistEntityIdent> = Vec::with_capacity(2);
        outputs.push(cache.persist_ident(assets.ident));
        outputs.push(cache.persist_ident(metadata.ident));

        atry!(
            bincode::serialize_into(&mut f_cache, &outputs);
            ["failed to serialize bincode data into `{}`", p_cache.display()]
        );

        // And that's it!

        atry!(
            f_cache.flush();
            ["failed to flush file `{}`", p_cache.display()]
        );
        Ok(())
    }
}
