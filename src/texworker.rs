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
    process::{Command, Stdio},
    sync::mpsc::{channel, TryRecvError},
};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_error, tt_warning, StatusBackend};
use threadpool::ThreadPool;
use walkdir::DirEntry;

use crate::worker_status::WorkerStatusBackend;

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

pub trait WorkerDriver: Default {
    /// The type that will be returned to the driver thread.
    type Item: Send + 'static;

    /// Initialize arguments/settings for the subcommand that will be run, which
    /// is a re-execution of the calling process.
    fn init_command(&self, cmd: &mut Command, entry: &DirEntry);

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
) -> Result<W::Item, WorkerError<()>> {
    // This function is run in a fresh thread, so it needs to create its own
    // status backend if it wants to report any information (because our status
    // system is not thread-safe). It also needs to do that to provide context
    // about the origin of any messages. It should fully report out any errors
    // that it encounters.
    let mut status =
        Box::new(WorkerStatusBackend::new(entry.path().display())) as Box<dyn StatusBackend>;

    let mut cmd = Command::new(&self_path);
    driver.init_command(&mut cmd, &entry);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tt_error!(status, "failed to relaunch self as TeX worker"; e.into());
            return Err(WorkerError::General(()));
        }
    };

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
        (true, WorkerError::Specific(_)) => Ok(driver.finish()), // <= the default
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

pub fn process_inputs<W: WorkerDriver, F>(
    mut cb: F,
    status: &mut dyn StatusBackend,
) -> Result<usize>
where
    F: FnMut(W::Item),
{
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

        let tx = tx.clone();
        let sp = self_path.clone();

        pool.execute(move || {
            let driver = W::default();
            tx.send(process_one_input(driver, sp, entry))
                .expect("channel waits for pool result");
        });
        n_tasks += 1;

        // Deal with results as we're doing the walk, if there are any.

        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(item) => {
                        cb(item);
                    }

                    Err(WorkerError::General(_)) => {
                        n_failures += 1;
                        tt_error!(status, "giving up early");
                        break; // give up
                    }

                    Err(WorkerError::Specific(_)) => {
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
        match result {
            Ok(item) => {
                cb(item);
            }

            // At this point, we've already launched anything, so we can't give
            // up early anymore; and the child process or inner callback should
            // have displayed the error.
            Err(_) => {
                n_failures += 1;
            }
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
