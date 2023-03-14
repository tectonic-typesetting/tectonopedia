// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! A stage of the build process that does a pass over all TeX inputs.
//!
//! - Have to launch TeX in subprocesses because the engine can't be multithreaded
//! - Use a threadpool to manage that
//! - Subprocess stderr is passed straight on through for error reporing
//! - Subprocess stdout is parsed for information transfer

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{ChildStdin, Command, Stdio},
    sync::mpsc::{channel, TryRecvError},
};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_error, tt_warning, StatusBackend};
use threadpool::ThreadPool;

use crate::{
    cache::{Cache, OpCacheData},
    index::IndexCollection,
    operation::{DigestData, RuntimeEntityIdent},
    stry,
    worker_status::WorkerStatusBackend,
};

#[derive(Debug)]
pub enum WorkerError<T> {
    /// Some kind of environmental error not specific to this particular input.
    /// We should abort the whole build because other jobs are probably going to
    /// fail too.
    General(T),

    /// An error specific to this input. We'll fail this input, but keep on
    /// going overall to report as many problems as we can.
    Specific(T),
}

impl<T> WorkerError<T> {
    pub fn map<F, U>(self, func: F) -> WorkerError<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            WorkerError::General(t) => WorkerError::General(func(t)),
            WorkerError::Specific(t) => WorkerError::Specific(func(t)),
        }
    }
}

pub trait WorkerResultExt<T> {
    fn unwrap_for_worker(self) -> Result<T>;
}

impl<T> WorkerResultExt<T> for Result<T, WorkerError<Error>> {
    fn unwrap_for_worker(self) -> Result<T> {
        match self {
            Ok(v) => Ok(v),

            Err(WorkerError::General(e)) => {
                println!("pedia:general-error");
                Err(e)
            }

            Err(WorkerError::Specific(e)) => Err(e),
        }
    }
}

/// Try something that returns an OldError, and report a General error if it fails.
#[macro_export]
macro_rules! ogtry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: OldError = e;
                return Err(WorkerError::General(SyncError::new(typecheck).into()));
            }
        }
    };
}

/// Try something that returns a new Error, and report a General error if it fails.
#[macro_export]
macro_rules! gtry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: Error = e.into();
                return Err(WorkerError::General(typecheck));
            }
        }
    };
}

/// Try something that returns an OldError, and report a Specific error if it fails.
#[macro_export]
macro_rules! ostry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: OldError = e;
                return Err(WorkerError::Specific(SyncError::new(typecheck).into()));
            }
        }
    };
}

/// Try something that returns a new Error, and report a Specific error if it fails.
#[macro_export]
macro_rules! stry {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let typecheck: Error = e.into();
                return Err(WorkerError::Specific(typecheck));
            }
        }
    };
}

/// A type that can drive a TeX worker process.
///
/// This type is created in the primary thread and sent to one of the threadpool
/// worker threads. In that worker thread, it then interacts closely with the
/// TeX subprocess. Once that subprocess exits, it is consumed.
pub trait WorkerDriver: Send {
    /// Get a digest uniquely identifying this processing task.
    ///
    /// The digest will be used to check in the build cache and see if this
    /// operation can actually be skipped.
    fn operation_ident(&self) -> DigestData;

    /// Initialize arguments/settings for the subcommand that will be run, which
    /// is a re-execution of the calling process.
    ///
    /// *task_num* is index number of this particular processing task.
    fn init_command(&self, cmd: &mut Command, task_num: usize);

    /// Send information to the subcommand over its standard input.
    fn send_stdin(&self, stdin: &mut ChildStdin) -> Result<()>;

    /// Process a line of output emitted by the worker process.
    fn process_output_record(&mut self, line: &str, status: &mut dyn StatusBackend);

    /// Finalize this processing operation.
    ///
    /// The result type returns information for the operation cache. This method
    /// is only called if the child process exits successfully.
    fn finish(self) -> Result<OpCacheData, WorkerError<Error>>;
}

fn process_one_input<W: WorkerDriver>(
    mut driver: W,
    self_path: PathBuf,
    n_tasks: usize,
    mut status: Box<dyn StatusBackend>,
) -> Result<OpCacheData, WorkerError<()>> {
    // This function should fully report out any errors that it encounters,
    // since it can only propagate a stateless flag as to whether a "specific"
    // or "general" error occurred; it can't propagate out detailed information.

    let mut cmd = Command::new(&self_path);
    driver.init_command(&mut cmd, n_tasks);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tt_error!(status, "failed to relaunch self as TeX worker"; e.into());
            return Err(WorkerError::General(()));
        }
    };

    // First, send input over stdin. It will be closed when we drop the handle.

    {
        let mut stdin = child.stdin.take().unwrap();

        if let Err(e) = driver.send_stdin(&mut stdin) {
            tt_error!(status, "failed to send input to TeX worker"; e.into());
            return Err(WorkerError::Specific(()));
        }
    }

    // Now read results from stdout.

    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut error_type = WorkerError::Specific(());

    for line in stdout.lines() {
        match line {
            Ok(line) => {
                if let Some(rest) = line.strip_prefix("pedia:") {
                    match rest {
                        "general-error" => {
                            error_type = WorkerError::General(());
                        }
                        _ => {
                            driver.process_output_record(rest, status.as_mut());
                        }
                    }
                } else {
                    tt_warning!(status.as_mut(), "unexpected stdout content: {}", line);
                }
            }

            Err(e) => {
                tt_warning!(status.as_mut(), "error reading worker stdout"; e.into());
            }
        }
    }

    // Wait for the process to finish and wrap up.

    let ec = match child.wait() {
        Ok(c) => c,
        Err(e) => {
            tt_error!(status, "failed to wait() for TeX worker"; e.into());
            return Err(error_type);
        }
    };

    match (ec.success(), &error_type) {
        (true, WorkerError::Specific(_)) => {} // this is the default case
        (true, WorkerError::General(_)) => {
            tt_warning!(
                status.as_mut(),
                "TeX worker had a successful exit code but reported failure"
            );
            return Err(error_type);
        }
        (false, _) => return Err(error_type),
    }

    driver.finish().map_err(|e| {
        e.map(|inner| {
            tt_error!(status, "error finalizing results"; inner);
            ()
        })
    })
}

/// A type that can manage the execution of a large batch of TeX processing jobs.
pub trait TexProcessor {
    /// This associated type is the one that actually deals with managing a TeX
    /// subprocess. Workers are created in the main thread and sent to a
    /// threadpool worker thread, in which they interact with the TeX
    /// subprocess. After that subprocess exits, the worker is consumed and its
    /// `Item` type is returned to the main thread.
    type Worker: WorkerDriver + 'static;

    /// Create a worker object.
    ///
    /// This function is called in the main thread. The worker will be sent to a
    /// worker thread and then drive a TeX subprocess.
    fn make_worker(
        &mut self,
        input: RuntimeEntityIdent,
        indices: &mut IndexCollection,
    ) -> Result<Self::Worker, WorkerError<Error>>;
}

pub fn process_inputs<'a, R: TexProcessor>(
    inputs: impl IntoIterator<Item = &'a RuntimeEntityIdent>,
    red: &mut R,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    status: &mut dyn StatusBackend,
) -> Result<usize> {
    let self_path = atry!(
        std::env::current_exe();
        ["cannot obtain the path to the current executable"]
    );

    let n_workers = 8; // !! make generic
    let pool = ThreadPool::new(n_workers);

    let (tx, rx) = channel();
    let mut n_tasks = 0;
    let mut n_failures = 0;

    for input in inputs {
        let input = *input;

        // Set up a custom status reporter for this input path.
        let mut item_status = {
            let path = indices.relpath_for_tex_source(input).unwrap();
            Box::new(WorkerStatusBackend::new(path)) as Box<dyn StatusBackend + Send>
        };

        let maybe_info = match process_input_prep(red, input, cache, indices, item_status.as_mut())
        {
            Ok(w) => w,

            Err(WorkerError::General(e)) => {
                n_failures += 1;
                tt_error!(item_status, "prep failed"; e);
                tt_error!(status, "giving up early");
                break; // give up
            }

            Err(WorkerError::Specific(e)) => {
                // TODO: if, say, 3 of the first 5 builds fail, give up
                // the whole shebang, under the assumption that
                // something is messed up that will break all of the
                // builds.
                n_failures += 1;
                tt_error!(item_status, "prep failed"; e);
                continue;
            }
        };

        if let Some(driver) = maybe_info {
            let tx = tx.clone();
            let sp = self_path.clone();

            pool.execute(move || {
                tx.send(process_one_input(driver, sp, n_tasks, item_status))
                    .expect("channel waits for pool result");
            });
            n_tasks += 1;
        } else {
            // We can use the cached results for this task.
            continue;
        }

        // Deal with results as we're doing the walk, if there are any.

        match rx.try_recv() {
            Ok(result) => {
                let ocd = match result {
                    Ok(ocd) => ocd,

                    Err(WorkerError::General(_)) => {
                        n_failures += 1;
                        tt_error!(status, "giving up early");
                        break; // give up
                    }

                    Err(WorkerError::Specific(_)) => {
                        // TODO: if, say, 3 of the first 5 builds fail, give up
                        // the whole shebang, under the assumption that
                        // something is messed up that will break all of the
                        // builds.
                        n_failures += 1;
                        continue;
                    }
                };

                process_input_finish(ocd, cache, indices, status);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => unreachable!(),
        }
    }

    drop(tx);

    for result in rx.iter() {
        let ocd = match result {
            Ok(ocd) => ocd,

            Err(_) => {
                // At this point, we've already launched everything, so we can't
                // give up early anymore; and the child process or inner callback
                // should have displayed the error.
                n_failures += 1;
                continue;
            }
        };

        process_input_finish(ocd, cache, indices, status);
    }

    ensure!(
        n_failures == 0,
        "{} out of {} build inputs failed",
        n_failures,
        n_tasks
    );

    Ok(n_tasks)
}

fn process_input_prep<R: TexProcessor>(
    red: &mut R,
    input: RuntimeEntityIdent,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    status: &mut dyn StatusBackend,
) -> Result<Option<R::Worker>, WorkerError<Error>> {
    let driver = red.make_worker(input, indices)?;
    let opid = driver.operation_ident();

    if !stry!(cache.operation_needs_rerun(&opid, indices, status)) {
        tt_warning!(status, "skipping input `{:?}`!!!", input);
        return Ok(None);
    }

    Ok(Some(driver))
}

fn process_input_finish(
    ocd: OpCacheData,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    status: &mut dyn StatusBackend,
) {
    // Since any failure only involves the caching step, not the actaul build
    // operation, we'll report it but not flag the error at a higher level.
    if let Err(e) = cache.finalize_operation(ocd, indices) {
        tt_error!(status, "failed to save caching information for a build step"; e);
    }
}
