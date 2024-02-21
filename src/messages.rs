// Copyright 2024 the Tectonic Project
// Licensed under the MIT License

//! Messages that describe the progress of build operations.
//!
//! These messages are used by the "watch UI" and `build` CLI
//! to update the user on how the build is going.

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
    BuildStarted,
    YarnOutput(YarnOutputMessage),
    ServerQuitting,
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
