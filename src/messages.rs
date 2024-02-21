// Copyright 2024 the Tectonic Project
// Licensed under the MIT License

//! Messages that describe the progress of build operations.
//!
//! These messages are used by the "watch UI" and `build` CLI
//! to update the user on how the build is going.

use std::sync::{Arc, Mutex};
use tectonic_status_base::{tt_note, StatusBackend};

/// A trait for types that can distribute messages
pub trait MessageBus: Clone + Send {
    async fn post(&mut self, msg: &Message);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
    BuildStarted,
    BuildComplete(BuildCompleteMessage),
    YarnOutput(YarnOutputMessage),
    ServerQuitting,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BuildCompleteMessage {
    pub elapsed: f32,
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
            Message::BuildComplete(d) => {
                tt_note!(
                    self.status.lock().unwrap(),
                    "full build took {:.1} seconds",
                    d.elapsed
                );
            }

            _ => {}
        }
    }
}
