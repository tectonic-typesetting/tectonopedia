// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

use clap::{Args, Parser, Subcommand};
use std::{
    sync::mpsc::{channel, TryRecvError},
    time::Instant,
};
use tectonic::status::termcolor::TermcolorStatusBackend;
use tectonic_errors::prelude::*;
use tectonic_status_base::{tt_error, tt_note, ChatterLevel, StatusBackend};
use threadpool::ThreadPool;

mod config;
mod inputs;
mod pass1;
mod worker_status;

use worker_status::WorkerStatusBackend;

fn main() {
    let args = ToplevelArgs::parse();

    let mut status = match &args.action {
        Action::FirstPassImpl(a) => {
            Box::new(WorkerStatusBackend::new(&a.tex_path)) as Box<dyn StatusBackend>
        }
        _ => Box::new(TermcolorStatusBackend::new(ChatterLevel::Normal)) as Box<dyn StatusBackend>,
    };

    if let Err(e) = args.exec(status.as_mut()) {
        status.report_error(&e);
        std::process::exit(1)
    }
}

#[derive(Debug, Parser)]
struct ToplevelArgs {
    #[command(subcommand)]
    action: Action,
}

impl ToplevelArgs {
    fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        match self.action {
            Action::Build(a) => a.exec(status),
            Action::FirstPassImpl(a) => a.exec(status),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Action {
    Build(BuildArgs),
    FirstPassImpl(pass1::FirstPassImplArgs),
}

#[derive(Args, Debug)]
struct BuildArgs {
    #[arg(long)]
    sample: Option<String>,
}

impl BuildArgs {
    fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        let t0 = Instant::now();

        let self_path = atry!(
            std::env::current_exe();
            ["cannot obtain the path to the current executable"]
        );

        let n_workers = 8; // !! make generic
        let pool = ThreadPool::new(n_workers);
        let (tx, rx) = channel();
        let mut n_tasks = 0;
        let mut n_failures = 0;

        for entry in inputs::InputIterator::new() {
            let entry = atry!(
                entry;
                ["error while walking input tree"]
            );

            let tx = tx.clone();
            let sp = self_path.clone();

            pool.execute(move || {
                tx.send(pass1::build_one_input(sp, entry))
                    .expect("channel waits for pool result");
            });
            n_tasks += 1;

            // Deal with results as we're doing the walk, if there are any.

            match rx.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(_) => {}

                        Err(pass1::FirstPassError::General(_)) => {
                            n_failures += 1;
                            tt_error!(status, "giving up early");
                            break; // give up
                        }

                        Err(pass1::FirstPassError::Specific(_)) => {
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
                Ok(_) => {}

                // At this point, we've already launched anything, so we can't
                // give up early anymore.
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

        tt_note!(status, "pass 1: processed {} inputs", n_tasks);

        // Indexing goes here!

        tt_note!(
            status,
            "build took {:.1} seconds",
            t0.elapsed().as_secs_f32()
        );
        Ok(())
    }
}
