// Copyright 2024 the Tectonic Project
// Licensed under the MIT License

//! The long-running "watch" operation.
//!
//! Here we monitor the source tree and rebuild on the fly, using Parcel as a
//! webserver to host the Pedia webapp in development mode. We run a *second*
//! webapp to report outputs from the build process, since there's so much going
//! on. This program runs a Websockets server that feeds information to the
//! build-info app.

use clap::Args;
use notify_debouncer_mini::{new_debouncer, notify, DebounceEventHandler, DebounceEventResult};
use std::{path::Path, time::Duration};
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;

use crate::yarn;

/// The watch operation.
#[derive(Args, Debug)]
pub struct WatchArgs {
    #[arg(long, short = 'j', default_value_t = 0)]
    parallel: usize,
}

impl WatchArgs {
    pub fn exec(self, _status: &mut dyn StatusBackend) -> Result<()> {
        let watcher = Watcher {
            parallel: self.parallel,
        };

        self.finish_exec(watcher)
    }

    fn finish_exec(self, watcher: Watcher) -> Result<()> {
        let mut yarn_child = yarn::yarn_serve()?;

        let mut debouncer = atry!(
            new_debouncer(Duration::from_millis(300), None, watcher);
            ["failed to set up filesystem change notifier"]
        );

        for dname in &["cls", "idx", "src", "txt", "web"] {
            atry!(
                debouncer
                    .watcher()
                    .watch(Path::new(dname), notify::RecursiveMode::Recursive);
                ["failed to watch directory `{}`", dname]
            );
        }

        let _ignore = yarn_child.wait();
        Ok(())
    }
}

struct Watcher {
    parallel: usize,
}

impl DebounceEventHandler for Watcher {
    fn handle_event(&mut self, event: DebounceEventResult) {
        if let Err(e) = event {
            eprintln!("fs watch error!");
        } else {
            println!("event!");
        }
    }
}
