// Copyright 2024 the Tectonic Project
// Licensed under the MIT License

//! The long-running "serve" operation.
//!
//! Here we monitor the source tree and rebuild on the fly, using Parcel as a
//! webserver to host the Pedia webapp in development mode. We run a *second*
//! webapp to report outputs from the build process, since there's so much going
//! on. This program runs a web server that hosts the build-info UI app as well
//! as a WebSocket service that allows communications between the backend and
//! the frontend.

use clap::Args;
use futures::{FutureExt, StreamExt};
use notify_debouncer_mini::{new_debouncer, notify, DebounceEventHandler, DebounceEventResult};
use std::{convert::Infallible, path::Path, sync::Arc, time::Duration};
use tectonic_errors::prelude::*;
use tectonic_status_base::StatusBackend;
use tokio::{
    process::{Child, Command},
    sync::{mpsc, oneshot, Mutex},
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{
    ws::{Message, WebSocket},
    Filter, Rejection, Reply,
};

/// The serve operation.
#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, short = 'j', default_value_t = 0)]
    parallel: usize,
}

/// A message to be delivered to the main "serve" thread.
enum ServeMessage {
    /// Quit.
    Quit,
}

impl ServeArgs {
    pub fn exec(self, _status: &mut dyn StatusBackend) -> Result<()> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(self.inner())
    }

    async fn inner(self) -> Result<()> {
        // A channel for the secondary tasks to issue commands to the main task.

        let (command_tx, mut command_rx) = mpsc::channel(1);

        // Set up filesystem change watching

        let (notify_tx, mut notify_rx) = mpsc::channel(1);

        let watcher = Watcher { notify_tx };

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

        // Set up the build-UI server

        let warp_state: WarpServer = Arc::new(Mutex::new(WarpServerState::new(command_tx.clone())));

        let ws_route = warp::path("ws")
            .and(warp::path::end())
            .and(warp::ws())
            .and(with_warp_state(warp_state.clone()))
            .and_then(ws_handler);

        let static_route = warp::fs::dir("serve-ui/dist").map(|reply: warp::filters::fs::File| {
            match reply.path().extension().and_then(|osstr| osstr.to_str()) {
                Some("css") => {
                    warp::reply::with_header(reply, "Content-Type", "text/css").into_response()
                }
                Some("html") => {
                    warp::reply::with_header(reply, "Content-Type", "text/html").into_response()
                }
                Some("js") => warp::reply::with_header(reply, "Content-Type", "text/javascript")
                    .into_response(),
                _ => reply.into_response(),
            }
        });

        let routes = ws_route
            .or(static_route)
            .with(warp::cors().allow_any_origin());

        let (warp_shutdown_tx, warp_shutdown_rx) = oneshot::channel();

        println!("serve UI listening on http://localhost:8000/");

        // Set up `yarn serve` for the app

        let (yarn_quit_tx, yarn_quit_rx) = mpsc::channel(1);

        let yarn_server = YarnServer::new(1234, yarn_quit_rx)?;
        println!("app listening on http://localhost:1234/");

        // Dispatch loop?

        let (addr, server) =
            warp::serve(routes).bind_with_graceful_shutdown(([127, 0, 0, 1], 8000), async {
                warp_shutdown_rx.await.ok();
            });

        let warp_join = tokio::task::spawn(server);
        let yarn_join = tokio::task::spawn(yarn_server.serve());

        while let Some(cmd) = command_rx.recv().await {
            match cmd {
                ServeMessage::Quit => {
                    println!("Quitting ...");
                    break;
                }
            }

            //tokio::select! {
            //    x = notify_rx.recv() => {
            //        println!("got notify: {x:?}");
            //        for c in &*warp_state.lock().await.clients {
            //            let _ = c.sender.send(Ok(Message::text("notify")));
            //        }
            //    },
            //}
        }

        // Shutdown

        warp_shutdown_tx.send(());

        yarn_quit_tx.send(()).await;
        warp_join.await;
        yarn_join.await;

        Ok(())
    }
}

// The webserver for the build/watch UI

struct Client {
    pub sender: mpsc::UnboundedSender<std::result::Result<Message, warp::Error>>,
}

struct WarpServerState {
    clients: Vec<Client>,
    command_tx: mpsc::Sender<ServeMessage>,
}

impl WarpServerState {
    pub fn new(command_tx: mpsc::Sender<ServeMessage>) -> Self {
        WarpServerState {
            clients: Vec::new(),
            command_tx,
        }
    }
}

type WarpServer = Arc<Mutex<WarpServerState>>;

fn with_warp_state(
    warp_state: WarpServer,
) -> impl Filter<Extract = (WarpServer,), Error = Infallible> + Clone {
    warp::any().map(move || warp_state.clone())
}

type WarpResult<T> = std::result::Result<T, Rejection>;

async fn ws_handler(ws: warp::ws::Ws, warp_state: WarpServer) -> WarpResult<impl Reply> {
    Ok(ws.on_upgrade(move |socket| ws_client_connection(socket, warp_state)))
}

async fn ws_client_connection(ws: WebSocket, warp_state: WarpServer) {
    // The outbound and inbound sides of the websocket.
    let (client_ws_tx, mut client_ws_rx) = ws.split();

    // A channel that we'll use to distribute outbound messages to the WS client.
    let (client_outbound_tx, client_outbound_rx) = mpsc::unbounded_channel();

    // Remember the sender
    let command_tx = {
        let mut state = warp_state.lock().await;

        state.clients.push(Client {
            sender: client_outbound_tx,
        });

        state.command_tx.clone()
    };

    // Spawn a task that just hangs out forwarding messages from the channel to
    // the WS client.
    let client_outbound_stream = UnboundedReceiverStream::new(client_outbound_rx);

    tokio::task::spawn(client_outbound_stream.forward(client_ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("error sending websocket message: {}", e);
        }
    }));

    // Meanwhile, we spend the rest of our time listening for client messages

    while let Some(result) = client_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("error receiving WebSocket message from client: {}", e);
                break;
            }
        };

        match msg.to_str() {
            Ok("quit") => {
                println!("got quit message");
                command_tx.send(ServeMessage::Quit).await;
            }

            _ => {
                eprintln!("unrecognized message: {:?}", msg);
            }
        }
    }
}

/// The filesystem change notification watcher. When a notification happens, all
/// we do is post a message onto our channel so that we can expose the event to
/// async-land.
struct Watcher {
    notify_tx: mpsc::Sender<()>,
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

// The `yarn serve` task

struct YarnServer {
    child: Child,
    quit_rx: mpsc::Receiver<()>,
}

impl YarnServer {
    fn new(port: u16, quit_rx: mpsc::Receiver<()>) -> Result<Self> {
        let mut cmd = Command::new("yarn");
        cmd.arg("serve")
            .arg(format!("--port={port}"))
            .arg("--watch-for-stdin")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());

        let child = atry!(
            cmd.spawn();
            ["failed to launch `yarn serve` process"]
        );

        Ok(YarnServer { child, quit_rx })
    }

    async fn serve(mut self) {
        let stdin = self.child.stdin.take().expect("failed to open child stdin");

        loop {
            tokio::select! {
                _ = self.quit_rx.recv() => {
                    break;
                },

                status = self.child.wait() => {
                    eprintln!("`yarn serve` process exited early: {status:?}");
                    break;
                }
            }
        }

        std::mem::drop(stdin);
        let status = self.child.wait().await;
        eprintln!("second yarn serve status: {status:?}");
    }
}
