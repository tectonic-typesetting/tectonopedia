// Copyright 2024 the Tectonic Project
// Licensed under the MIT License

//! Messages that describe the progress of build operations.
//!
//! These messages are used by the "watch UI" and `build` CLI
//! to update the user on how the build is going.

use std::sync::{Arc, Mutex};
use tectonic_status_base::{tt_error, tt_note, StatusBackend};

/// A trait for types that can distribute messages
pub trait MessageBus: Clone + Send {
    async fn post(&mut self, msg: &Message);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
    /// A build process has started. In "serve" mode this will happen
    /// unpredictably, when filesystem changes are observed, but the next build
    /// will only start after the previous one ends.
    BuildStarted,

    /// A build process has completed. Maybe successfully, maybe not.
    BuildComplete(BuildCompleteMessage),

    /// A new phase of the build process has started. Any previous phase can now
    /// be considered complete. The string value is a kebab-case, user-facing
    /// name for the build phase.
    PhaseStarted(String),

    /// Some kind of sub-command is being invoked as part of the tool process.
    /// The string value is the command in shell-like syntax; it is only
    /// informational, so we don't try to convey its arguments in full
    /// correctness.
    CommandLaunched(String),

    /// An error has been encountered during the build. These errors are not
    /// related to the TeX compilation and so are not associated with any
    /// particular input file.
    Error(ErrorMessage),

    /// Output from the `yarn serve` program has been received.
    YarnOutput(YarnOutputMessage),

    /// The "serve" mode server is exiting.
    ServerQuitting,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BuildCompleteMessage {
    pub success: bool,
    pub elapsed: f32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ErrorMessage {
    /// The essential message
    pub message: String,

    /// Additional contextual information, advice, etc.
    pub context: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct YarnOutputMessage {
    pub stream: ToolOutputStream,
    pub lines: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputStream {
    Stdout,
    Stderr,
}

/// A MessageBus that uses the Tectonic StatusBackend mechanism
/// to report status updates in a way fitting for CLI usage.
#[derive(Clone)]
pub struct CliStatusMessageBus {
    status: Arc<Mutex<Box<dyn StatusBackend + Send>>>,
}

impl CliStatusMessageBus {
    //pub fn new(status: Box<dyn StatusBackend + Send>) -> Self {
    //    let status = Arc::new(Mutex::new(status));
    //    CliStatusMessageBus { status }
    //}

    /// Temporary function for transitioning to this system.
    pub fn new_scaffold(status: Arc<Mutex<Box<dyn StatusBackend + Send>>>) -> Self {
        CliStatusMessageBus { status }
    }

    //pub fn into_inner(self) -> Box<dyn StatusBackend + Send> {
    //    let status = Arc::into_inner(self.status).unwrap();
    //    let status = Mutex::into_inner(status).unwrap();
    //    status
    //}
}

impl MessageBus for CliStatusMessageBus {
    async fn post(&mut self, msg: &Message) {
        match msg {
            Message::CommandLaunched(d) => {
                tt_note!(self.status.lock().unwrap(), "running `{d}`");
            }

            Message::BuildComplete(d) => {
                tt_note!(
                    self.status.lock().unwrap(),
                    "full build took {:.1} seconds",
                    d.elapsed
                );
            }

            Message::Error(d) => {
                let mut s = self.status.lock().unwrap();

                tt_error!(s, "{}", d.message);

                for c in &d.context[..] {
                    tt_error!(s, "  {c}");
                }
            }

            _ => {}
        }
    }
}
