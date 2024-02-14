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
use notify_debouncer_mini::{DebounceEventHandler, DebounceEventResult};
use std::{collections::HashMap, convert::Infallible, sync::Arc};
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
    pub fn exec(self, _status: &mut dyn StatusBackend) -> Result<()> {
        let watcher = Watcher {
            parallel: self.parallel,
        };

        let clients: Clients = Arc::new(Mutex::new(Vec::new()));

        let ws_route = warp::path("ws")
            .and(warp::ws())
            .and(with_clients(clients.clone()))
            .and_then(ws_handler);

        let routes = ws_route.with(warp::cors().allow_any_origin());

        println!("Listening http://127.0.0.1:8000/");

        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(warp::serve(routes).run(([127, 0, 0, 1], 8000)));

        Ok(())
    }
}

async fn ws_handler(ws: warp::ws::Ws, clients: Clients) -> WarpResult<impl Reply> {
    Ok(ws.on_upgrade(move |socket| client_connection(socket, clients)))
}

async fn client_connection(ws: WebSocket, clients: Clients) {
    let (client_ws_sender, _client_ws_rcv) = ws.split();
    let (client_sender, client_rcv) = mpsc::unbounded_channel();

    let client_rcv = UnboundedReceiverStream::new(client_rcv);
    clients.lock().await.push(Client {
        sender: client_sender,
    });

    tokio::task::spawn(client_rcv.forward(client_ws_sender).map(|result| {
        if let Err(e) = result {
            eprintln!("error sending websocket msg: {}", e);
        }
    }));
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
