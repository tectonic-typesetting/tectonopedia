// Copyright 2022 the Tectonic Project
// Licensed under the MIT License.

//! Status reporting for the TeX workers. This translates messages to our
//! async/parallel-friendly MessageBus scheme and emits them as one-line JSON
//! messages for convenient transfer from the workers to the main process. The
//! implementation has a lot in common with the SyncMessageBusSender.

use tectonic_errors::Error;
use tectonic_status_base::{MessageKind, StatusBackend};

use crate::messages::{AlertMessage, Message};

pub struct WorkerStatusBackend {
    context: String,
}

impl WorkerStatusBackend {
    pub fn new<C: ToString>(context: C) -> WorkerStatusBackend {
        let context = context.to_string();
        WorkerStatusBackend { context }
    }

    fn post(&mut self, msg: Message) {
        println!("pedia-msg:{}", serde_json::to_string(&msg).unwrap());
    }
}

impl StatusBackend for WorkerStatusBackend {
    fn report(&mut self, kind: MessageKind, args: std::fmt::Arguments<'_>, err: Option<&Error>) {
        let mut alert = AlertMessage {
            file: Some(self.context.clone()),
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
        // TODO: might actually want/need to handle this for real?
        self.post(Message::Error(AlertMessage {
            file: None,
            message: "(internal error: TeX error log should not get here)".into(),
            context: Default::default(),
        }));
    }
}
