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
use futures::{FutureExt, StreamExt};
use notify_debouncer_mini::{new_debouncer, notify, DebounceEventHandler, DebounceEventResult};
use std::{convert::Infallible, path::Path, sync::Arc, time::Duration};
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{
    ws::{Message, WebSocket},
    Filter, Rejection, Reply,
};

/// The watch operation.
#[derive(Args, Debug)]
pub struct WatchArgs {
    #[arg(long, short = 'j', default_value_t = 0)]
    parallel: usize,
}

struct Client {
    pub sender: mpsc::UnboundedSender<std::result::Result<Message, warp::Error>>,
}

type Clients = Arc<Mutex<Vec<Client>>>;

fn with_clients(clients: Clients) -> impl Filter<Extract = (Clients,), Error = Infallible> + Clone {
    warp::any().map(move || clients.clone())
}

type WarpResult<T> = std::result::Result<T, Rejection>;

impl WatchArgs {
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(self.wrapper(status));
        Ok(())
    }

    async fn wrapper(self, status: &mut dyn StatusBackend) {
        if let Err(e) = self.inner().await {
            status.report_error(&e);
            std::process::exit(1)
        }
    }

    async fn inner(self) -> Result<()> {
        let clients: Clients = Arc::new(Mutex::new(Vec::new()));

        // Set up filesystem change watching

        let (notify_tx, mut notify_rx) = mpsc::channel(1);

        let watcher = Watcher {
            notify_tx,
            _parallel: self.parallel,
        };

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

        // Set up the WebSocket server

        let ws_route = warp::path("ws")
            .and(warp::ws())
            .and(with_clients(clients.clone()))
            .and_then(ws_handler);

        let routes = ws_route.with(warp::cors().allow_any_origin());

        println!("build data WebSocket backend listening on ws://127.0.0.1:8000/ws");

        // Dispatch loop?

        tokio::task::spawn(warp::serve(routes).run(([127, 0, 0, 1], 8000)));

        loop {
            tokio::select! {
                x = notify_rx.recv() => {
                    println!("got notify: {x:?}");
                    for c in &*clients.lock().await {
                        c.sender.send(Ok(Message::text("notify")));
                    }
                },
            }
        }

        Ok(())
    }
}

async fn ws_handler(ws: warp::ws::Ws, clients: Clients) -> WarpResult<impl Reply> {
    Ok(ws.on_upgrade(move |socket| client_connection(socket, clients)))
}

async fn client_connection(ws: WebSocket, clients: Clients) {
    // The outbound and inbound sides of the websocket.
    let (client_ws_send, _client_ws_recv) = ws.split();

    // A channel that we'll use to distribute outbound messages to the WS client.
    let (client_outbound_send, client_outbound_recv) = mpsc::unbounded_channel();

    // Remember the sender
    clients.lock().await.push(Client {
        sender: client_outbound_send,
    });

    // Spawn a task that just hangs out forwarding messages from the channel to
    // the WS client.
    let client_outbound_stream = UnboundedReceiverStream::new(client_outbound_recv);

    tokio::task::spawn(
        client_outbound_stream
            .forward(client_ws_send)
            .map(|result| {
                if let Err(e) = result {
                    eprintln!("error sending websocket message: {}", e);
                }
            }),
    );
}

struct Watcher {
    notify_tx: mpsc::Sender<()>,
    _parallel: usize,
}

impl DebounceEventHandler for Watcher {
    fn handle_event(&mut self, event: DebounceEventResult) {
        if let Err(_e) = event {
            eprintln!("fs watch error!");
        } else {
            println!("event!");
            futures::executor::block_on(async { self.notify_tx.send(()).await.unwrap() });
        }
    }
}
