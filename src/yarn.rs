// Copyright 2023 the Tectonic Project
// Licensed under the MIT License

//! Running the `yarn` tool.

use tectonic_errors::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, oneshot},
};

use crate::{
    messages::{Message, MessageBus, ToolOutputMessage, ToolOutputStream},
    serve::ServeCommand,
};

/// Run a `yarn` command.
async fn do_yarn<T: MessageBus>(command: &str, mut bus: T, piped: bool) -> Result<()> {
    let mut cmd = Command::new("yarn");
    cmd.arg(command);

    if piped {
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }

    bus.post(Message::CommandLaunched(format!("yarn {}", command)))
        .await;

    let mut child = atry!(
        cmd.spawn();
        ["failed to launch `yarn {}` process", command]
    );

    if !piped {
        println!();
    } else {
        let stdout = child.stdout.take().expect("failed to open child stdout");
        let mut stdout_lines = BufReader::new(stdout).lines();

        let stderr = child.stderr.take().expect("failed to open child stderr");
        let mut stderr_lines = BufReader::new(stderr).lines();

        loop {
            tokio::select! {
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            bus.post(Message::ToolOutput(ToolOutputMessage {
                                stream: ToolOutputStream::Stdout,
                                lines: vec![line],
                            })).await;
                        }

                        Err(e) => {
                            bus.error::<String, _>(None, "failed to read child process stdout", Some(e.into())).await;
                        }

                        _ => {}
                    }
                }

                line = stderr_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            bus.post(Message::ToolOutput(ToolOutputMessage {
                                stream: ToolOutputStream::Stderr,
                                lines: vec![line],
                            })).await;
                        }

                        Err(e) => {
                            bus.error::<String, _>(None, "failed to read child process stderr", Some(e.into())).await;
                        }

                        _ => {}
                    }
                }

                _ = child.wait() => {
                    break;
                }
            }
        }
    }

    let ec = child.wait().await;

    if !piped {
        println!();
    }

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
pub async fn yarn_index<T: MessageBus>(bus: T, piped: bool) -> Result<()> {
    do_yarn("index", bus, piped).await
}

/// Run the `yarn build` command.
pub async fn yarn_build<T: MessageBus>(bus: T, piped: bool) -> Result<()> {
    do_yarn("build", bus, piped).await
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
        let mut cmd = Command::new("yarn");
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
                            self.bus.post(Message::YarnServeOutput(ToolOutputMessage {
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
                            self.bus.post(Message::YarnServeOutput(ToolOutputMessage {
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
