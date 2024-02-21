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
    signal::unix::{signal, SignalKind},
    sync::{mpsc, oneshot, Mutex},
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{
    ws::{Message as WsMessage, WebSocket},
    Filter, Rejection, Reply,
};

use crate::{
    messages::{Message, MessageBus},
    yarn::YarnServer,
};

/// The serve operation.
#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, short = 'j', default_value_t = 0)]
    parallel: usize,
}

/// A message to be delivered to the main "serve" thread.
pub enum ServeCommand {
    /// Rebuild the document.
    Build,

    /// Quit the whole application.
    Quit(Result<()>),
}

enum ServeOutcome {
    Ok,
    Signal(libc::c_int),
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

        let (command_tx, mut command_rx) = mpsc::channel(4);

        // Set up filesystem change watching

        let watcher = Watcher {
            command_tx: command_tx.clone(),
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

        // Set up the build-UI server

        let mut clients: WarpClientCollection = Arc::new(Mutex::new(Vec::new()));

        let warp_state: WarpState = Arc::new(Mutex::new(WarpStateInner::new(command_tx.clone())));

        let ws_route = warp::path("ws")
            .and(warp::path::end())
            .and(warp::ws())
            .and(with_warp_state(clients.clone(), warp_state.clone()))
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

        let (warp_quit_tx, warp_quit_rx) = oneshot::channel();

        let (warp_addr, warp_server) =
            warp::serve(routes).bind_with_graceful_shutdown(([127, 0, 0, 1], 8000), async {
                warp_quit_rx.await.ok();
            });

        // Set up `yarn serve` for the app

        let (yarn_quit_tx, yarn_quit_rx) = oneshot::channel();
        let yarn_server = YarnServer::new(1234, yarn_quit_rx, command_tx.clone(), clients.clone())?;

        // Signal handling

        let mut sigint_event = atry!(
            signal(SignalKind::interrupt());
            ["failed to register SIGINT handler"]
        );

        let mut sighup_event = atry!(
            signal(SignalKind::hangup());
            ["failed to register SIGHUP handler"]
        );

        let mut sigterm_event = atry!(
            signal(SignalKind::terminate());
            ["failed to register SIGTERM handler"]
        );

        // Start it all up!

        let warp_join = tokio::task::spawn(warp_server);
        let yarn_join = tokio::task::spawn(yarn_server.serve());

        println!();
        println!("    app listening on:        http://localhost:1234/");
        println!(
            "    build UI listening on:   http://localhost:{}/",
            warp_addr.port()
        );
        println!();

        // Our main loop -- watch for incoming commands and build when needed.

        let mut outcome = Err(anyhow!("unexpected mainloop termination"));

        loop {
            tokio::select! {
                cmd = command_rx.recv() => {
                    if let Some(cmd) = cmd {
                        match cmd {
                            ServeCommand::Quit(r) => {
                                match r {
                                    Ok(_) => {
                                        println!("Quitting as commanded ...");
                                        outcome = Ok(ServeOutcome::Ok);
                                    }

                                    Err(e) => {
                                        outcome = Err(e);
                                    }
                                }

                                break;
                            }

                            ServeCommand::Build => {
                                println!("Should build now.");
                                clients.post(&Message::BuildStarted).await;
                            }
                        }
                    } else {
                        break;
                    }
                }

                _ = sigint_event.recv() => {
                    println!("\nQuitting on SIGINT ...");
                    // See below for an explanation of what's going on here:
                    outcome = Ok(ServeOutcome::Signal(libc::SIGINT));
                    break;
                }

                _ = sighup_event.recv() => {
                    println!("\nQuitting on SIGHUP ...");
                    outcome = Ok(ServeOutcome::Signal(libc::SIGHUP));
                    break;
                }

                _ = sigterm_event.recv() => {
                    println!("\nQuitting on SIGTERM ...");
                    outcome = Ok(ServeOutcome::Signal(libc::SIGTERM));
                    break;
                }
            }
        }

        // Shutdown

        clients.post(&Message::ServerQuitting).await;

        if let Err(_) = yarn_quit_tx.send(()) {
            eprintln!("error: failed to send shutdown signal to the `yarn serve` subprocess");
        } else if let Err(e) = yarn_join.await {
            eprintln!("error waiting for `yarn serve` subprocess to finish: {e}");
        }

        if let Err(_) = warp_quit_tx.send(()) {
            eprintln!("error: failed to send shutdown signal to the Warp webserver task");
        } else if let Err(e) = warp_join.await {
            eprintln!("error waiting for Warp webserver task to finish: {e}");
        }

        match outcome {
            Ok(ServeOutcome::Ok) => Ok(()),

            Ok(ServeOutcome::Signal(signum)) => {
                // When we're terminated by a signal, we want to clean up
                // nicely, but then we want to indicate the killing signal to
                // our parent process. The correct way to do this is to restore
                // the associated signal handler to its default behavior, and
                // then re-kill ourselves with the same signal.
                unsafe {
                    libc::signal(signum, libc::SIG_DFL);
                    libc::kill(libc::getpid(), signum);
                }
                // Anything from here on out should be unreachable, but just in case ...
                Err(anyhow!(format!(
                    "failed to self-terminate with signal {signum}"
                )))
            }

            Err(e) => Err(e),
        }
    }
}

// The message bus for Warp-powered websocket clients

struct Client {
    pub sender: mpsc::UnboundedSender<std::result::Result<WsMessage, warp::Error>>,
}

type WarpClientCollection = Arc<Mutex<Vec<Client>>>;

impl MessageBus for WarpClientCollection {
    async fn post(&mut self, msg: &Message) {
        let ws_msg = WsMessage::text(serde_json::to_string(msg).unwrap());

        for c in &*self.lock().await {
            // Assume that failure is a disconnected client; just ignore it??
            let _ = c.sender.send(Ok(ws_msg.clone()));
        }
    }
}

// The webserver for the build/watch UI

struct WarpStateInner {
    command_tx: mpsc::Sender<ServeCommand>,
}

impl WarpStateInner {
    pub fn new(command_tx: mpsc::Sender<ServeCommand>) -> Self {
        WarpStateInner { command_tx }
    }
}

type WarpState = Arc<Mutex<WarpStateInner>>;

fn with_warp_state(
    clients: WarpClientCollection,
    warp_state: WarpState,
) -> impl Filter<Extract = ((WarpClientCollection, WarpState),), Error = Infallible> + Clone {
    warp::any().map(move || (clients.clone(), warp_state.clone()))
}

type WarpResult<T> = std::result::Result<T, Rejection>;

async fn ws_handler(
    ws: warp::ws::Ws,
    (clients, warp_state): (WarpClientCollection, WarpState),
) -> WarpResult<impl Reply> {
    Ok(ws.on_upgrade(move |socket| ws_client_connection(socket, clients, warp_state)))
}

async fn ws_client_connection(ws: WebSocket, clients: WarpClientCollection, warp_state: WarpState) {
    // The outbound and inbound sides of the websocket.
    let (client_ws_tx, mut client_ws_rx) = ws.split();

    // A channel that we'll use to distribute outbound messages to the WS client.
    let (client_outbound_tx, client_outbound_rx) = mpsc::unbounded_channel();

    // Record this client
    clients.lock().await.push(Client {
        sender: client_outbound_tx,
    });

    // Spawn a task that just hangs out forwarding messages from the channel to
    // the WS client.
    let client_outbound_stream = UnboundedReceiverStream::new(client_outbound_rx);

    tokio::task::spawn(client_outbound_stream.forward(client_ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("error sending websocket message: {}", e);
        }
    }));

    // Meanwhile, we spend the rest of our time listening for client messages

    let command_tx = warp_state.lock().await.command_tx.clone();

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
                if let Err(e) = command_tx.send(ServeCommand::Quit(Ok(()))).await {
                    println!("error in WebSocket client handler notifying main task: {e:?}");
                    break;
                }
            }

            _ => {
                eprintln!("unrecognized WebSocket client message: {:?}", msg);
            }
        }
    }

    // TODO: remove ourselves from the client pool in some fashion so that
    // the server doesn't try to keep on sending messages to us.
}

/// The filesystem change notification watcher. When a notification happens, all
/// we do is post a message onto our channel so that we can expose the event to
/// async-land.
struct Watcher {
    command_tx: mpsc::Sender<ServeCommand>,
}

impl DebounceEventHandler for Watcher {
    fn handle_event(&mut self, event: DebounceEventResult) {
        if let Err(_e) = event {
            eprintln!("fs watch error!");
        } else {
            futures::executor::block_on(async {
                self.command_tx.send(ServeCommand::Build).await.unwrap()
            });
        }
    }
}
