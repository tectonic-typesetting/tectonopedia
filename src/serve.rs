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
use notify_debouncer_mini::{
    new_debouncer, notify, DebounceEventHandler, DebounceEventResult, DebouncedEventKind,
};
use std::{
    collections::HashSet,
    convert::Infallible,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};
use tectonic_errors::{anyhow::Context, prelude::*};
use tectonic_status_base::{tt_note, StatusBackend};
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::{mpsc, oneshot, Mutex},
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use walkdir::WalkDir;
use warp::{
    ws::{Message as WsMessage, WebSocket},
    Filter, Rejection, Reply,
};

use crate::{
    build::build_through_index,
    messages::{BuildCompleteMessage, Message, MessageBus, ServerInfoMessage},
    yarn::YarnServer,
};

/// The serve operation.
#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, short = 'o')]
    open: bool,

    #[arg(long, short = 'j', default_value_t = 0)]
    parallel: usize,
}

/// A message to be delivered to the main "serve" thread.
pub enum ServeCommand {
    /// Rebuild the document.
    Build,

    /// Notify the server that a client has connectd.
    ClientConnected,

    /// Quit the whole application.
    Quit(Result<()>),
}

enum ServeOutcome {
    Ok,
    Signal(libc::c_int),
}

impl ServeArgs {
    pub fn exec(self, status: &mut dyn StatusBackend) -> Result<()> {
        setup_prerequisites(status)?;

        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(self.inner())
    }

    async fn inner(self) -> Result<()> {
        let n_workers = if self.parallel > 0 {
            self.parallel
        } else {
            num_cpus::get()
        };

        // A channel for the secondary tasks to issue commands to the main task.

        let (command_tx, mut command_rx) = mpsc::channel(8);

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
            warp::serve(routes).bind_with_graceful_shutdown(([127, 0, 0, 1], 5678), async {
                warp_quit_rx.await.ok();
            });

        // Set up `yarn serve` for the app

        let yarn_serve_port = 1234;
        let (yarn_quit_tx, yarn_quit_rx) = oneshot::channel();
        let yarn_server = YarnServer::new(
            yarn_serve_port,
            yarn_quit_rx,
            command_tx.clone(),
            clients.clone(),
        )?;

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

        let app_url = format!("http://localhost:{yarn_serve_port}/");
        let ui_url = format!("http://localhost:{}/", warp_addr.port());

        println!();
        println!("    app listening on:        {app_url}");
        println!("    build UI listening on:   {ui_url}");
        println!();

        let info_message = ServerInfoMessage {
            app_port: yarn_serve_port,
            n_workers,
        };

        // Open in the browser, maybe

        if self.open {
            if let Err(e) = open::that_detached(&ui_url) {
                eprintln!("failed to open UI in web browser: {e}");
            }
        }

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

                            ServeCommand::ClientConnected => {
                                clients.post(Message::ServerInfo(info_message.clone())).await;

                                // Trigger a build (awkwardly). In the standard
                                // setup, this means that we'll only kick off
                                // the build when the web UI is ready to accept
                                // messages about the build progress. This will
                                // re-build if multiple clients connect, but who
                                // cares?
                                if let Err(e) = command_tx.send(ServeCommand::Build).await {
                                    eprintln!("error: failed to send internal command: {e}");
                                }
                            }

                            ServeCommand::Build => {
                                clients.post(Message::BuildStarted).await;

                                match build_through_index(n_workers, true, clients.clone()).await {
                                    Ok((t0, changed)) => {
                                        if let Err(e) = update_serve_dir(changed) {
                                            clients.error::<String, _>(None, format!("unable to update `serve`directory"), Some(e.into())).await;
                                        }

                                        // FIXME! Always post build-complete
                                        clients.post(Message::BuildComplete(BuildCompleteMessage {
                                            success: true,
                                            elapsed: t0.elapsed().as_secs_f32(),
                                        }))
                                        .await;
                                    }

                                    Err(e) => clients.error::<String, _>(None, "build failure", Some(e)).await
                                }
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

        clients.post(Message::ServerQuitting).await;

        if yarn_quit_tx.send(()).is_err() {
            eprintln!("error: failed to send shutdown signal to the `yarn serve` subprocess");
        } else if let Err(e) = yarn_join.await {
            eprintln!("error waiting for `yarn serve` subprocess to finish: {e}");
        }

        if warp_quit_tx.send(()).is_err() {
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

fn setup_prerequisites(status: &mut dyn StatusBackend) -> Result<()> {
    // `yarn install` in the main directory?

    if !Path::new("node_modules").exists() {
        tt_note!(status, "running one-time `yarn install` ...");

        println!();
        let mut cmd = Command::new("yarn");
        cmd.arg("install");

        let status = atry!(
            cmd.status();
            ["failed to spawn `yarn install` subcommand"]
        );

        println!();

        if !status.success() {
            bail!("`yarn install` subcommand failed");
        }
    }

    // `yarn install` in the serve-ui sub directory?

    let mut pb = PathBuf::from("serve-ui");
    pb.push("node_modules");

    if !pb.exists() {
        tt_note!(status, "running one-time `yarn install` in `serve-ui` ...");

        println!();
        let mut cmd = Command::new("yarn");
        cmd.arg("install");

        // see docs for current_dir()
        pb.pop();
        let pb = atry!(
            pb.canonicalize();
            ["failed to canonicalize subdir"]
        );

        cmd.current_dir(pb);

        let status = atry!(
            cmd.status();
            ["failed to spawn `yarn install` subcommand"]
        );

        println!();

        if !status.success() {
            bail!("`yarn install` subcommand failed");
        }
    }

    // Build the serve UI?

    let mut pb = PathBuf::from("serve-ui");
    pb.push("dist");
    pb.push("index.html");

    if !pb.exists() {
        tt_note!(status, "running one-time `yarn build` in `serve-ui` ...");

        println!();
        let mut cmd = Command::new("yarn");
        cmd.arg("build");

        // see docs for current_dir()
        pb.pop();
        pb.pop();
        let pb = atry!(
            pb.canonicalize();
            ["failed to canonicalize subdir"]
        );

        cmd.current_dir(pb);

        let status = atry!(
            cmd.status();
            ["failed to spawn `yarn build` subcommand"]
        );

        println!();

        if !status.success() {
            bail!("`yarn build` subcommand failed");
        }
    }

    // stub `_all.html` for the `yarn serve` process?

    let mut pb = PathBuf::from("serve");
    pb.push("_all.html");

    if !pb.exists() {
        // This one doesn't need reporting to the user

        let mut parent = pb.clone();
        parent.pop();
        atry!(
            std::fs::create_dir_all(&parent);
            ["failed to create directory hierarchy `{}`", parent.display()]
        );

        let mut f = atry!(
            std::fs::File::create(&pb);
            ["failed to create `{}`", pb.display()]
        );

        atry!(
            f.write_all(b"<html><head><title>Nothing Yet</title></head><body>Nothing here yet.</body></html>\n");
            ["failed to write to `{}`", pb.display()]
        );
    }

    // Ready to go!

    tt_note!(status, "starting servers ...");
    Ok(())
}

fn update_serve_dir(mut changed: Vec<String>) -> Result<()> {
    // FIXME: should make this a stringinterner probably

    let mut changed_set = HashSet::new();

    for relpath in changed.drain(..) {
        // These paths do not have the `build/` prefix
        changed_set.insert(relpath);
    }

    // Identify the files that need updating and duplicate them.

    let prefix = format!("build{}", std::path::MAIN_SEPARATOR);
    let out_base = PathBuf::from("serve");
    let mut updates = Vec::new();

    fn stage_path(i: usize) -> String {
        format!(".stage{i}.tmp")
    }

    for entry in WalkDir::new("build").into_iter() {
        let entry = atry!(
            entry;
            ["failed to traverse build output directory"]
        );

        let relpath = match entry.path().to_str().and_then(|s| s.strip_prefix(&prefix)) {
            Some(s) => s,
            None => continue, // warn?
        };

        let in_md = atry!(
            entry.metadata();
            ["failed to probe path `{}`", entry.path().display()]
        );

        if in_md.is_dir() {
            continue;
        }

        let mut out_path = out_base.clone();
        out_path.push(relpath);

        let needs_copy = if changed_set.contains(relpath) {
            true
        } else {
            match std::fs::metadata(&out_path) {
                Ok(out_md) => {
                    in_md.file_type() != out_md.file_type() || in_md.len() != out_md.len()
                }

                Err(ref e) if e.kind() == ErrorKind::NotFound => true,

                Err(e) => {
                    return Err(e)
                        .context(format!("failed to probe path `{}`", out_path.display()));
                }
            }
        };

        if needs_copy {
            atry!(
                std::fs::create_dir_all(out_path.parent().unwrap());
                ["failed to create directories leading to `{}`", out_path.parent().unwrap().display()]
            );

            let sp = stage_path(updates.len());

            atry!(
                std::fs::copy(entry.path(), &sp);
                ["failed to copy `{}` to `{}`", entry.path().display(), sp]
            );

            updates.push(out_path);
        }
    }

    // Rename to finish the job as quickly as possible, aiming
    // to avoid extra rebuilds and potential I/O issues.

    for (i, out_path) in updates.drain(..).into_iter().enumerate() {
        let sp = stage_path(i);

        atry!(
            std::fs::rename(&sp, &out_path);
            ["failed to rename `{}` to `{}`", sp, out_path.display()]
        );
    }

    Ok(())
}

// The message bus for Warp-powered websocket clients

struct Client {
    pub sender: mpsc::UnboundedSender<std::result::Result<WsMessage, warp::Error>>,
}

type WarpClientCollection = Arc<Mutex<Vec<Client>>>;

impl MessageBus for WarpClientCollection {
    async fn post(&mut self, msg: Message) {
        let ws_msg = WsMessage::text(serde_json::to_string(&msg).unwrap());

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

    // Let the main thread know that we're here.

    let command_tx = warp_state.lock().await.command_tx.clone();

    if let Err(e) = command_tx.send(ServeCommand::ClientConnected).await {
        eprintln!("error in WebSocket client handler notifying main task: {e:?}");
    }

    // Meanwhile, we spend the rest of our time listening for client messages

    while let Some(result) = client_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("error receiving WebSocket message from client: {}", e);
                break;
            }
        };

        if msg.is_close() {
            break;
        }

        match msg.to_str() {
            Ok("trigger_build") => {
                if let Err(e) = command_tx.send(ServeCommand::Build).await {
                    eprintln!("error in WebSocket client handler notifying main task: {e:?}");
                    break;
                }
            }

            Ok("quit") => {
                if let Err(e) = command_tx.send(ServeCommand::Quit(Ok(()))).await {
                    eprintln!("error in WebSocket client handler notifying main task: {e:?}");
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
    fn handle_event(&mut self, result: DebounceEventResult) {
        match result {
            Ok(events) => {
                for event in &events {
                    // It appears that in typical cases, we'll get zero or more
                    // AnyContinuous events followed by an Any event once the
                    // updates finally stop. So, just ignore the former.
                    match event.kind {
                        DebouncedEventKind::Any => {
                            futures::executor::block_on(async {
                                self.command_tx.send(ServeCommand::Build).await.unwrap()
                            });
                            return;
                        }

                        _ => {}
                    }
                }
            }

            Err(errors) => {
                eprintln!("warning: filesystem change watch error(s):");
                for error in &errors {
                    eprintln!("  {error}");
                }
            }
        }
    }
}
