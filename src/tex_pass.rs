// Copyright 2022-2023 the Tectonic Project
// Licensed under the MIT License

//! A stage of the build process that does a pass over all TeX inputs.
//!
//! - Have to launch TeX in subprocesses because the engine can't be multithreaded
//! - Use a task pool to manage that
//! - Subprocess stderr is passed straight on through for error reporing
//! - Subprocess stdout is parsed for information transfer

use futures::Future;
use std::{path::PathBuf, time::Instant};
use tectonic_errors::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{ChildStdin, Command},
    sync::mpsc::{channel, error::TryRecvError},
};
use tokio_task_pool::Pool;

use crate::{
    cache::{Cache, OpCacheData},
    index::IndexCollection,
    messages::{
        bus_to_status, AlertMessage, BuildCompleteMessage, BuildStartedMessage,
        InputDebugOutputMessage, Message, MessageBus,
    },
    operation::{DigestData, RuntimeEntityIdent},
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
/// This type is created in the primary thread and sent to one of the task pool
/// workers. In that worker, it then interacts closely with the TeX subprocess.
/// Once that subprocess exits, it is consumed.
pub trait WorkerDriver: Send {
    /// This type encodes basic information about the TeX operation for this
    /// input.
    ///
    /// This type will be created by the processor and used to determine if the
    /// task needs to be rerun. If the task does need rerunning, it should be
    /// sent over to its worker driver and eventually returned by the
    /// [`Self::finish`] method. But if the cached task result is OK, the item
    /// will be processed at the top level without the worker ever being
    /// created.
    type OpInfo: TexOperation + 'static;

    /// Initialize arguments/settings for the subcommand that will be run, which
    /// is a re-execution of the calling process.
    fn init_command(&self, cmd: &mut Command);

    /// Send information to the subcommand over its standard input.
    fn send_stdin(&self, stdin: ChildStdin) -> impl Future<Output = Result<()>> + Send;

    /// Process a line of output emitted by the worker process.
    ///
    /// This function has a somewhat silly call signature because we want it to
    /// be able to report results via the bus infrastructure, but it cannot
    /// conveniently be made async with a sendable future due to its borrows.
    /// Since we process line-by-line, hopefully it's not a big limitation to
    /// only be able to produce a single message output.
    fn process_output_record(&mut self, line: &str) -> Option<Message>;

    /// Finalize this processing operation.
    ///
    /// The result type returns information for the operation cache and any data
    /// that should be aggregated by the processor. This method is only called
    /// if the child process exits successfully.
    fn finish(self) -> Result<(OpCacheData, Self::OpInfo), WorkerError<Error>>;
}

/// A wrapper for the real work function that makes sure to post
/// messages about when the build starts and stops.
async fn process_one_input<W: WorkerDriver, B: MessageBus>(
    driver: W,
    input_path: String,
    self_path: PathBuf,
    mut bus: B,
    debug: bool,
) -> Result<(OpCacheData, W::OpInfo), WorkerError<()>> {
    let t0 = Instant::now();
    bus.post(Message::BuildStarted(BuildStartedMessage {
        file: Some(input_path.clone()),
    }))
    .await;

    let result =
        process_one_input_inner(driver, input_path.clone(), self_path, bus.clone(), debug).await;

    bus.post(Message::BuildComplete(BuildCompleteMessage {
        file: Some(input_path),
        success: result.is_ok(),
        elapsed: t0.elapsed().as_secs_f32(),
    }))
    .await;

    result
}

async fn process_one_input_inner<W: WorkerDriver, B: MessageBus>(
    mut driver: W,
    input_path: String,
    self_path: PathBuf,
    mut bus: B,
    debug: bool,
) -> Result<(OpCacheData, W::OpInfo), WorkerError<()>> {
    // This function should fully report out any errors that it encounters,
    // since it can only propagate a stateless flag as to whether a "specific"
    // or "general" error occurred; it can't propagate out detailed information.

    let mut cmd = Command::new(self_path);

    driver.init_command(&mut cmd);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());

    if debug {
        cmd.arg("--debug");
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            bus.error(
                Some(input_path),
                "failed to relaunch self as TeX worker",
                Some(e.into()),
            )
            .await;
            return Err(WorkerError::General(()));
        }
    };

    // First, send input over stdin. It will be closed when we drop the handle.

    {
        let stdin = child.stdin.take().unwrap();

        if let Err(e) = driver.send_stdin(stdin).await {
            bus.error(
                Some(input_path),
                "failed to send input to TeX worker",
                Some(e),
            )
            .await;
            return Err(WorkerError::Specific(()));
        }
    }

    // Now read results from stdout.

    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut error_type = WorkerError::Specific(());

    loop {
        let line = stdout.next_line().await;

        match line {
            Ok(Some(line)) => {
                if let Some(rest) = line.strip_prefix("pedia-msg:") {
                    match serde_json::from_str(rest) {
                        Ok(msg) => bus.post(msg).await,
                        Err(e) => {
                            bus.warning(
                                Some(input_path.clone()),
                                format!("failed to parse JSON message from child: {}", rest),
                                Some(e.into()),
                            )
                            .await
                        }
                    }
                } else if let Some(rest) = line.strip_prefix("pedia:") {
                    match rest {
                        "general-error" => {
                            error_type = WorkerError::General(());
                        }
                        _ => {
                            if let Some(msg) = driver.process_output_record(rest) {
                                bus.post(msg).await;
                            }
                        }
                    }
                } else if debug {
                    bus.post(Message::InputDebugOutput(InputDebugOutputMessage {
                        file: input_path.clone(),
                        lines: vec![line],
                    }))
                    .await;
                } else {
                    bus.warning(
                        Some(input_path.clone()),
                        format!("unexpected stdout content: {}", line),
                        None,
                    )
                    .await;
                }
            }

            Ok(None) => break,

            Err(e) => {
                bus.warning(
                    Some(input_path.clone()),
                    "error reading worker stdout",
                    Some(e.into()),
                )
                .await;
                break;
            }
        }
    }

    // Wait for the process to finish and wrap up.

    let ec = match child.wait().await {
        Ok(c) => c,
        Err(e) => {
            bus.error(
                Some(input_path),
                "failed to wait() for TeX worker",
                Some(e.into()),
            )
            .await;
            return Err(error_type);
        }
    };

    match (ec.success(), &error_type) {
        (true, WorkerError::Specific(_)) => {} // this is the default case
        (true, WorkerError::General(_)) => {
            bus.warning(
                Some(input_path),
                "TeX worker had a successful exit code but reported failure",
                None,
            )
            .await;
            return Err(error_type);
        }
        (false, _) => return Err(error_type),
    }

    match driver.finish() {
        Ok(t) => Ok(t),

        Err(e) => match e {
            WorkerError::General(e) => {
                bus.error(Some(input_path), "error finalizing results", Some(e))
                    .await;
                Err(WorkerError::General(()))
            }

            WorkerError::Specific(e) => {
                bus.error(Some(input_path), "error finalizing results", Some(e))
                    .await;
                Err(WorkerError::Specific(()))
            }
        },
    }
}

/// A type that can manage the execution of a large batch of TeX processing jobs.
pub trait TexProcessor {
    /// This associated type is the one that actually deals with managing a TeX
    /// subprocess. Workers are created in the main thread and sent to a worker
    /// task, in which they interact with the TeX subprocess. After that
    /// subprocess exits, the worker is consumed and its `Item` type is returned
    /// to the main thread.
    type Worker: WorkerDriver + 'static;

    /// Get a user-facing description for this processing stage.
    fn description(&self) -> &'static str;

    /// Create an operation info object.
    ///
    /// This function is called in the main thread. The information will be used
    /// to determine whether this particular job needs to be rerun. If so, a worker
    /// will be created and will take ownership of the info.
    fn make_op_info(
        &mut self,
        input: RuntimeEntityIdent,
        cache: &mut Cache,
        indices: &mut IndexCollection,
    ) -> Result<<Self::Worker as WorkerDriver>::OpInfo>;

    /// Create a worker object.
    ///
    /// This function is called in the main thread. The worker will be sent to a
    /// worker thread and then drive a TeX subprocess.
    fn make_worker(
        &mut self,
        opinfo: <Self::Worker as WorkerDriver>::OpInfo,
        indices: &mut IndexCollection,
    ) -> Result<Self::Worker, WorkerError<Error>>;

    /// Accumulate information about an operation.
    ///
    /// This function is called on the main thread with a [`WorkerDriver::OpInfo`]
    /// value. The value may or may not have been passed through a
    /// [`Self::Worker`], depending on whether the cache indicated that the
    /// operation actually needed to be rerun or not.
    fn accumulate_output(
        &mut self,
        opinfo: <Self::Worker as WorkerDriver>::OpInfo,
        was_rerun: bool,
    );
}

pub trait TexOperation: Send {
    /// Get a digest uniquely identifying this processing task.
    ///
    /// The digest will be used to check in the build cache and see if this
    /// operation can actually be skipped.
    fn operation_ident(&self) -> DigestData;
}

pub async fn process_inputs<'a, P: TexProcessor, B: MessageBus + 'static>(
    inputs: impl IntoIterator<Item = &'a RuntimeEntityIdent>,
    n_workers: usize,
    proc: &mut P,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    mut bus: B,
) -> Result<usize> {
    let self_path = atry!(
        std::env::current_exe();
        ["cannot obtain the path to the current executable"]
    );

    let pool = Pool::bounded(n_workers);

    let (tx, mut rx) = channel(2 * n_workers);
    let mut n_tasks = 0;
    let mut n_failures = 0;

    for input in inputs {
        let input = *input;
        let input_path = indices.relpath_for_tex_source(input).unwrap().to_owned();

        // In principle this could/should be a WorkerError, but the distinction
        // doesn't seem super important.
        let opinfo = atry!(
            proc.make_op_info(input, cache, indices);
            ["failed to prepare operation for input `{}`", indices.relpath_for_tex_source(input).unwrap()]
        );

        let opid = opinfo.operation_ident();

        // If the cache query fails, that's definitely something that should
        // cause us to bail immediately.
        let needs_rerun = atry!(
            bus_to_status(bus.clone(), |s| cache.operation_needs_rerun(&opid, indices, s)).await;
            ["failed to query build cache"]
        );

        if !needs_rerun {
            bus.post(Message::BuildStarted(BuildStartedMessage {
                file: Some(input_path.clone()),
            }))
            .await;

            bus.post(Message::Note(AlertMessage {
                file: Some(input_path.clone()),
                message: format!("{}: skipping build - output is cached", proc.description()),
                context: vec![],
            }))
            .await;

            bus.post(Message::BuildComplete(BuildCompleteMessage {
                file: Some(input_path),
                success: true,
                elapsed: 0.,
            }))
            .await;

            // If we're not going to fire off a thread to process this task,
            // we just accumulate it into the results directly and we're done.
            proc.accumulate_output(opinfo, false);
            continue;
        }

        // If we're still here, it looks like we actually need to launch
        // a TeX job for this input.

        let driver = match proc.make_worker(opinfo, indices) {
            Ok(d) => d,

            Err(WorkerError::General(e)) => {
                n_failures += 1;
                bus.error(
                    Some(input_path.clone()),
                    "giving up early".to_owned(),
                    Some(e),
                )
                .await;
                break; // give up
            }

            Err(WorkerError::Specific(e)) => {
                // TODO: if, say, 3 of the first 5 builds fail, give up
                // the whole shebang, under the assumption that
                // something is messed up that will break all of the
                // builds.
                n_failures += 1;
                bus.error(
                    Some(input_path.clone()),
                    "giving up early".to_owned(),
                    Some(e),
                )
                .await;

                // By `continue`-ing here, we are discarding the opinfo and
                // not including this input in any subsequent processing.
                // That's OK since we'll abandon the build after this pass.
                continue;
            }
        };

        let tx = tx.clone();
        let sp = self_path.clone();
        let bc = bus.clone();
        let ip = input_path.clone();

        pool.spawn(async move {
            tx.send(process_one_input(driver, ip, sp, bc, false).await)
                .await
                .expect("channel waits for pool result");
        })
        .await
        .expect("failed to launch TeX worker");
        n_tasks += 1;

        // Deal with results as we're doing the walk, if there are any.

        match rx.try_recv() {
            Ok(result) => {
                let tup = match result {
                    Ok(tup) => tup,

                    Err(WorkerError::General(_)) => {
                        n_failures += 1;
                        bus.post(Message::Error(AlertMessage {
                            file: Some(input_path.clone()),
                            message: "giving up early".into(),
                            context: Default::default(),
                        }))
                        .await;
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

                process_input_finish(proc, tup.0, tup.1, cache, indices, bus.clone()).await;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => unreachable!(),
        }
    }

    // Handle all of the jobs that finish after we're done walking.

    drop(tx);

    while let Some(result) = rx.recv().await {
        let tup = match result {
            Ok(tup) => tup,

            Err(_) => {
                // At this point, we've already launched everything, so we can't
                // give up early anymore; and the child process or inner callback
                // should have displayed the error.
                n_failures += 1;
                continue;
            }
        };

        process_input_finish(proc, tup.0, tup.1, cache, indices, bus.clone()).await;
    }

    // OK, all done!

    ensure!(
        n_failures == 0,
        "{} out of {} build inputs failed",
        n_failures,
        n_tasks
    );

    Ok(n_tasks)
}

async fn process_input_finish<P: TexProcessor, B: MessageBus>(
    proc: &mut P,
    ocd: OpCacheData,
    item: <<P as TexProcessor>::Worker as WorkerDriver>::OpInfo,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    mut bus: B,
) {
    proc.accumulate_output(item, true);

    // Since any failure only involves the caching step, not the actual build
    // operation, we'll report it but not flag the error at a higher level.
    if let Err(e) = cache.finalize_operation(ocd, indices) {
        bus.error::<String, _>(
            None,
            "failed to save caching information for a build step",
            Some(e),
        )
        .await;
    }
}

/// This will only work with pass1 right now, since we haven't bothered to
/// implement --debug for the pass2 subprocess.
pub async fn debug_one_input<'a, P: TexProcessor, B: MessageBus + 'static>(
    input: RuntimeEntityIdent,
    proc: &mut P,
    cache: &mut Cache,
    indices: &mut IndexCollection,
    bus: B,
) -> Result<()> {
    let self_path = atry!(
        std::env::current_exe();
        ["cannot obtain the path to the current executable"]
    );

    let input_path = indices.relpath_for_tex_source(input).unwrap().to_owned();

    let opinfo = atry!(
        proc.make_op_info(input, cache, indices);
        ["failed to prepare operation for input `{}`", indices.relpath_for_tex_source(input).unwrap()]
    );

    let driver = match proc.make_worker(opinfo, indices) {
        Ok(d) => d,
        Err(WorkerError::General(e)) => return Err(e),
        Err(WorkerError::Specific(e)) => return Err(e),
    };

    match process_one_input(driver, input_path.clone(), self_path, bus.clone(), true).await {
        Ok(_) => Ok(()),
        Err(WorkerError::General(_)) => {
            bail!("failed to process the input, for a reason unrelated to its particular content")
        }
        Err(WorkerError::Specific(_)) => bail!("failed to process the input"),
    }
}
