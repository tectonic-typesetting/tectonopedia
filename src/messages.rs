// Copyright 2024 the Tectonic Project
// Licensed under the MIT License

//! Messages that describe the progress of build operations.
//!
//! These messages are used by the "serve UI" and `build` CLI
//! to update the user on how the build is going.
//!
//! See also /serve-ui/src/messages.ts, which defines TypeScript types
//! mirroring these.

use futures::Future;
use std::sync::{Arc, Mutex};
use tectonic_errors::Error;
use tectonic_status_base::{tt_error, tt_note, tt_warning, MessageKind, StatusBackend};
use tokio::sync::mpsc;

/// A trait for types that can distribute messages
pub trait MessageBus: Clone + Send {
    fn post(&mut self, msg: Message) -> impl Future<Output = ()> + Send;

    fn error<T1: ToString, T2: ToString>(
        &mut self,
        file: Option<T1>,
        message: T2,
        err: Option<Error>,
    ) -> impl Future<Output = ()> + Send {
        let alert = AlertMessage::new(file, message, err);
        self.post(Message::Error(alert))
    }

    fn warning<T1: ToString, T2: ToString>(
        &mut self,
        file: Option<T1>,
        message: T2,
        err: Option<Error>,
    ) -> impl Future<Output = ()> + Send {
        let alert = AlertMessage::new(file, message, err);
        self.post(Message::Warning(alert))
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
    /// A build process has started. In "serve" mode this will happen
    /// unpredictably, when filesystem changes are observed, but the next build
    /// will only start after the previous one ends.
    BuildStarted(BuildStartedMessage),

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

    /// A notable event has been encountered during the build.
    Note(AlertMessage),

    /// A warning has been encountered during the build.
    Warning(AlertMessage),

    /// An error has been encountered during the build.
    Error(AlertMessage),

    /// Output from a synchronously-run tool program has been received.
    ToolOutput(ToolOutputMessage),

    /// Output from the `yarn serve` program has been received.
    YarnServeOutput(ToolOutputMessage),

    /// Output from a file-specific debug operation has been received.
    InputDebugOutput(InputDebugOutputMessage),

    /// Information about the build server. Sent to clients on startup.
    ServerInfo(ServerInfoMessage),

    /// The "serve" mode server is exiting.
    ServerQuitting,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BuildStartedMessage {
    /// The source file that this message is associated with, if any.
    ///
    /// When the file is unspecified, this message refers to the
    /// overall build. Otherwise, it refers to a single source file
    /// that is part of the whole build.
    pub file: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BuildCompleteMessage {
    /// The source file that this message is associated with, if any.
    ///
    /// When the file is unspecified, this message refers to the
    /// overall build. Otherwise, it refers to a single source file
    /// that is part of the whole build.
    pub file: Option<String>,

    /// Whether the build was successful or not.
    pub success: bool,

    /// How long the build took, in seconds.
    pub elapsed: f32,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AlertMessage {
    /// The source file that this message is associated with, if any
    pub file: Option<String>,

    /// The essential message
    pub message: String,

    /// Additional contextual information, advice, etc.
    pub context: Vec<String>,
}

impl AlertMessage {
    pub fn new<T1: ToString, T2: ToString>(
        file: Option<T1>,
        message: T2,
        err: Option<Error>,
    ) -> Self {
        let mut alert = AlertMessage {
            file: file.map(|t| t.to_string()),
            message: message.to_string(),
            context: Default::default(),
        };

        if let Some(e) = err {
            for item in e.chain() {
                alert.context.push(item.to_string());
            }
        }

        alert
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ServerInfoMessage {
    /// The port at which the actual app is running. We express it as a port,
    /// rather than a full URL, in case someone is accessing us through some
    /// kind of proxy situation.
    pub app_port: u16,

    /// The number of workers used during the parallelized passes
    pub n_workers: usize,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ToolOutputMessage {
    pub stream: ToolOutputStream,
    pub lines: Vec<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputStream {
    Stdout,
    Stderr,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct InputDebugOutputMessage {
    pub file: String,
    pub lines: Vec<String>,
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
    async fn post(&mut self, msg: Message) {
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

            Message::Note(d) => {
                let mut s = self.status.lock().unwrap();

                tt_note!(s, "{}", d.message);

                for c in &d.context[..] {
                    tt_note!(s, "  {c}");
                }
            }

            Message::Warning(d) => {
                let mut s = self.status.lock().unwrap();

                tt_warning!(s, "{}", d.message);

                for c in &d.context[..] {
                    tt_warning!(s, "  {c}");
                }
            }

            Message::ToolOutput(d) => match d.stream {
                ToolOutputStream::Stdout => {
                    for line in &d.lines {
                        println!("{line}");
                    }
                }

                ToolOutputStream::Stderr => {
                    for line in &d.lines {
                        eprintln!("{line}");
                    }
                }
            },

            _ => {}
        }
    }
}

pub struct SyncMessageBusSender {
    tx: mpsc::Sender<Message>,
}

impl SyncMessageBusSender {
    pub fn post(&mut self, msg: Message) {
        self.tx.blocking_send(msg).unwrap();
    }

    pub fn file_error<T1: ToString, T2: ToString>(
        &mut self,
        file: T1,
        message: T2,
        err: Option<Error>,
    ) {
        let alert = AlertMessage::new(Some(file), message, err);
        self.post(Message::Error(alert))
    }
}

impl StatusBackend for SyncMessageBusSender {
    fn report(&mut self, kind: MessageKind, args: std::fmt::Arguments<'_>, err: Option<&Error>) {
        let mut alert = AlertMessage {
            file: None,
            message: format!("{}", args),
            context: Default::default(),
        };

        if let Some(e) = err {
            for item in e.chain() {
                alert.context.push(item.to_string());
            }
        }

        let msg = match kind {
            MessageKind::Note => Message::Note(alert),
            MessageKind::Warning => Message::Warning(alert),
            MessageKind::Error => Message::Error(alert),
        };

        self.post(msg)
    }

    fn dump_error_logs(&mut self, _output: &[u8]) {
        self.post(Message::Error(AlertMessage {
            file: None,
            message: "(internal error: TeX error log should not get here)".into(),
            context: Default::default(),
        }));
    }
}

pub struct SyncMessageBusReceiver {
    rx: mpsc::Receiver<Message>,
}

impl SyncMessageBusReceiver {
    pub async fn drain<T: MessageBus>(mut self, mut bus: T) {
        while let Some(msg) = self.rx.recv().await {
            bus.post(msg).await;
        }
    }
}

pub fn new_sync_bus_channel() -> (SyncMessageBusSender, SyncMessageBusReceiver) {
    let (tx, rx) = mpsc::channel(16);
    let send = SyncMessageBusSender { tx };
    let recv = SyncMessageBusReceiver { rx };
    (send, recv)
}

pub async fn bus_to_status<B: MessageBus, F, R>(bus: B, func: F) -> R
where
    F: FnOnce(&mut dyn StatusBackend) -> R,
{
    // todo? use an unbounded channel
    let (mut send, recv) = new_sync_bus_channel();
    let result = func(&mut send);
    std::mem::drop(send);
    recv.drain(bus).await;
    result
}
