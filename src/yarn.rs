// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Running the `yarn` tool.

use std::process::{Command, Stdio};
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_note, StatusBackend};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command as TokioCommand},
    sync::{mpsc, oneshot},
};

use crate::{
    messages::{Message, MessageBus, ToolOutputStream, YarnOutputMessage},
    serve::ServeCommand,
};

/// Run a `yarn` command.
fn do_yarn(command: &str, status: &mut dyn StatusBackend) -> Result<()> {
    let mut cmd = Command::new("yarn");
    cmd.arg(command);

    tt_note!(status, "running `yarn {}`:", command);
    println!();

    let ec = cmd.status();
    println!();

    let ec = atry!(
        ec;
        ["failed to execute the `yarn {}` subprocess", command]
    );

    if !ec.success() {
        bail!(
            "the `yarn {}` subprocess exited with an error code",
            command
        );
    }

    Ok(())
}

/// Run a `yarn` command, quietly.
fn do_yarn_quiet(command: &str) -> Result<()> {
    let mut cmd = Command::new("yarn");
    cmd.arg(command).stdout(Stdio::null());

    let ec = cmd.status();

    let ec = atry!(
        ec;
        ["failed to execute the `yarn {}` subprocess", command]
    );

    if !ec.success() {
        bail!(
            "the `yarn {}` subprocess exited with an error code",
            command
        );
    }

    Ok(())
}

/// Run the `yarn index` command.
///
/// This isn't plugged into the incremental build system (for now?). We just run
/// the command.
pub fn yarn_index(terse_output: bool, status: &mut dyn StatusBackend) -> Result<()> {
    if terse_output {
        do_yarn_quiet("index")
    } else {
        do_yarn("index", status)
    }
}

/// Run the `yarn build` command.
pub fn yarn_build(status: &mut dyn StatusBackend) -> Result<()> {
    do_yarn("build", status)
}

/// The `yarn serve` task for the development server
pub struct YarnServer<T: MessageBus> {
    child: Child,
    quit_rx: oneshot::Receiver<()>,
    command_tx: mpsc::Sender<ServeCommand>,
    bus: T,
}

impl<T: MessageBus> YarnServer<T> {
    pub fn new(
        port: u16,
        quit_rx: oneshot::Receiver<()>,
        command_tx: mpsc::Sender<ServeCommand>,
        bus: T,
    ) -> Result<Self> {
        let mut cmd = TokioCommand::new("yarn");
        cmd.arg("serve")
            .arg(format!("--port={port}"))
            .arg("--watch-for-stdin")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = atry!(
            cmd.spawn();
            ["failed to launch `yarn serve` process"]
        );

        Ok(YarnServer {
            child,
            quit_rx,
            command_tx,
            bus,
        })
    }

    pub async fn serve(mut self) {
        let stdin = self.child.stdin.take().expect("failed to open child stdin");

        let stdout = self
            .child
            .stdout
            .take()
            .expect("failed to open child stdout");
        let mut stdout_lines = BufReader::new(stdout).lines();

        let stderr = self
            .child
            .stderr
            .take()
            .expect("failed to open child stderr");
        let mut stderr_lines = BufReader::new(stderr).lines();

        loop {
            tokio::select! {
                _ = &mut self.quit_rx => {
                    break;
                },

                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            self.bus.post(&Message::YarnOutput(YarnOutputMessage {
                                stream: ToolOutputStream::Stdout,
                                lines: vec![line],
                            })).await;
                        }

                        Err(e) => {
                            eprintln!("error: failed to read `yarn serve` output: {e}");
                        }

                        _ => {}
                    }
                }

                line = stderr_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            self.bus.post(&Message::YarnOutput(YarnOutputMessage {
                                stream: ToolOutputStream::Stderr,
                                lines: vec![line],
                            })).await;
                        }

                        Err(e) => {
                            eprintln!("error: failed to read `yarn serve` output: {e}");
                        }

                        _ => {}
                    }
                }

                status = self.child.wait() => {
                    let msg = match status {
                        Ok(s) => format!("`yarn serve` subprocess exited early, {s}"),
                        Err(e) => format!("`yarn serve` subprocess exited early and failed to get outcome: {e}"),
                    };

                    let cmd = ServeCommand::Quit(Err(anyhow!(msg.clone())));

                    if let Err(e) = self.command_tx.send(cmd).await {
                        eprintln!("error: {msg}");
                        eprintln!("  ... furthermore, the yarn task failed to notify main task to exit: {e:?}");
                    }

                    break;
                }
            }
        }

        // Close stdin, informing the process to exit (thanks to `--watch-for-stdin`)
        std::mem::drop(stdin);

        let status = self.child.wait().await;

        if status.is_err() {
            eprintln!("error: `yarn serve` subprocess exited with an error: {status:?}");
        }
    }
}
