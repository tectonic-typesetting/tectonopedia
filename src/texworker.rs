// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! TeX workers.
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
use walkdir::DirEntry;

use crate::{
    cache::{Cache, OpCacheData},
    worker_status::WorkerStatusBackend,
    InputId,
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
/// TeX subprocess. Once that subprocess exits, it is consumed, and its
/// [`Self::Item`] value is then sent back to the main thread.
pub trait WorkerDriver: Send {
    /// The type that will be returned to the driver thread.
    ///
    /// Since a WorkerDriver must itself be `Send`, it might make sense for this
    /// associated type to be `Self`.
    type Item: Send + 'static;

    /// Initialize arguments/settings for the subcommand that will be run, which
    /// is a re-execution of the calling process.
    ///
    /// *entry* is the information about the input file. *task_num* is index
    /// number of this particular processing task.
    fn init_command(&self, cmd: &mut Command, entry: &DirEntry, task_num: usize);

    /// Send information to the subcommand over its standard input.
    fn send_stdin(&self, stdin: &mut ChildStdin) -> Result<()>;

    /// Process a line of output emitted by the worker process.
    fn process_output_record(&mut self, line: &str, status: &mut dyn StatusBackend);

    /// Finish processing, returning the value to be sent to the driver thread.
    /// Only called if the child process exits successfully.
    fn finish(self) -> Self::Item;
}

fn process_one_input<W: WorkerDriver>(
    mut driver: W,
    self_path: PathBuf,
    entry: DirEntry,
    id: InputId,
    n_tasks: usize,
) -> Result<(InputId, W::Item), WorkerError<()>> {
    // This function is run in a fresh thread, so it needs to create its own
    // status backend if it wants to report any information (because our status
    // system is not thread-safe). It also needs to do that to provide context
    // about the origin of any messages. It should fully report out any errors
    // that it encounters.
    let mut status =
        Box::new(WorkerStatusBackend::new(entry.path().display())) as Box<dyn StatusBackend>;

    let mut cmd = Command::new(&self_path);
    driver.init_command(&mut cmd, &entry, n_tasks);
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

    let ec = match child.wait() {
        Ok(c) => c,
        Err(e) => {
            tt_error!(status, "failed to wait() for TeX worker"; e.into());
            return Err(error_type);
        }
    };

    match (ec.success(), &error_type) {
        (true, WorkerError::Specific(_)) => Ok((id, driver.finish())), // <= the default
        (true, WorkerError::General(_)) => {
            tt_warning!(
                status.as_mut(),
                "TeX worker had a successful exit code but reported failure"
            );
            Err(error_type)
        }
        (false, _) => Err(error_type),
    }
}

/// A type that can manage the execution of a large batch of TeX processing jobs.
pub trait TexReducer {
    /// This associated type is the one that actually deals with managing a TeX
    /// subprocess. Workers are created in the main thread and sent to a
    /// threadpool worker thread, in which they interact with the TeX
    /// subprocess. After that subprocess exits, the worker is consumed and its
    /// `Item` type is returned to the main thread.
    type Worker: WorkerDriver + 'static;

    /// Assign an opaque input identifier to an input that is to be processed,
    /// and et up the operation associated with this task, which will let us
    /// decide if actually needs to be run or not.
    ///
    /// This function is called in the main thread.
    fn set_up_operation(
        &mut self,
        input_name: String,
        cache: &mut Cache,
    ) -> Result<(InputId, OpCacheData), WorkerError<Error>>;

    /// Create a worker object. This function is called in the main thread. The
    /// worker will be sent to a worker thread and then drive a TeX subprocess.
    fn make_worker(
        &mut self,
        id: InputId,
        ocd: OpCacheData,
        cache: &mut Cache,
    ) -> Result<Self::Worker, WorkerError<Error>>;

    /// This function must print out any error if one is encountered. Due to the
    /// parallelization approach, the returned result can indicate error
    /// information but not be used to report any information.
    fn process_item(
        &mut self,
        id: InputId,
        item: <Self::Worker as WorkerDriver>::Item,
        cache: &mut Cache,
    ) -> Result<(), WorkerError<()>>;
}

pub fn reduce_inputs<R: TexReducer>(
    red: &mut R,
    cache: &mut Cache,
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

    for entry in crate::inputs::InputIterator::new() {
        let entry = atry!(
            entry;
            ["error while walking input tree"]
        );

        let maybe_info = match reduce_input_prep(red, &entry, cache, status) {
            Ok(w) => w,

            Err(WorkerError::General(e)) => {
                n_failures += 1;
                tt_error!(status, "needs test failed for input `{}`; giving up early", entry.path().display(); e);
                break; // give up
            }

            Err(WorkerError::Specific(e)) => {
                // TODO: if, say, 3 of the first 5 builds fail, give up
                // the whole shebang, under the assumption that
                // something is messed up that will break all of the
                // builds.
                n_failures += 1;
                tt_error!(status, "needs test failed for input `{}`; trying others", entry.path().display(); e);
                continue;
            }
        };

        if let Some((id, driver)) = maybe_info {
            let tx = tx.clone();
            let sp = self_path.clone();

            pool.execute(move || {
                tx.send(process_one_input(driver, sp, entry, id, n_tasks))
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
                match result.and_then(|t| red.process_item(t.0, t.1, cache)) {
                    Ok(_) => {}

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
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => unreachable!(),
        }
    }

    drop(tx);

    for result in rx.iter() {
        if result
            .and_then(|t| red.process_item(t.0, t.1, cache))
            .is_err()
        {
            // At this point, we've already launched everything, so we can't
            // give up early anymore; and the child process or inner callback
            // should have displayed the error.
            n_failures += 1;
        }
    }

    ensure!(
        n_failures == 0,
        "{} out of {} build inputs failed",
        n_failures,
        n_tasks
    );

    Ok(n_tasks)
}

fn reduce_input_prep<R: TexReducer>(
    red: &mut R,
    entry: &DirEntry,
    cache: &mut Cache,
    status: &mut dyn StatusBackend,
) -> Result<Option<(InputId, R::Worker)>, WorkerError<Error>> {
    let (id, ocd) = red.set_up_operation(entry.path().display().to_string(), cache)?;

    if !stry!(cache.operation_needs_rerun(&ocd, status)) {
        tt_warning!(status, "skipping input `{}`!!!", entry.path().display());
        return Ok(None);
    }

    let driver = red.make_worker(id, ocd, cache)?;
    Ok(Some((id, driver)))
}
