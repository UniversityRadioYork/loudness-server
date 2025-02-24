use std::{collections::HashMap, sync::mpsc::Sender};

use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket}, Path, State, WebSocketUpgrade
    }, http::StatusCode, response::{Html, IntoResponse, Response}, routing::{any, get, post}, Json, Router
};
use serde::Serialize;
use tokio::sync::watch::Receiver;

use crate::{
    Command, InputsLoudness,
    config::{Config, InputConfig},
};

#[derive(Clone)]
struct AppState {
    config: Config,
    state: Receiver<InputsLoudness>,
    command_tx: Sender<Command>,
}

async fn events(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|socket| handle_events(socket, state))
}

async fn send_state(socket: &mut WebSocket, state: &InputsLoudness) -> bool {
    let payload = match serde_json::to_string(&state) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::error!(?e, "failed to serialize state");
            return false;
        }
    };

    if let Err(e) = socket.send(Message::text(payload)).await {
        tracing::error!(?e, "failed to send state");
        false
    } else {
        true
    }
}

async fn handle_events(mut socket: WebSocket, mut state: AppState) {
    {
        let current_state = state.state.borrow_and_update().clone();
        if !send_state(&mut socket, &current_state).await {
            return;
        }
    }

    loop {
        tokio::select! {
            Some(msg) = socket.recv() => {
                let Ok(msg) = msg else {
                    return;
                };

                let Ok(()) = (match msg {
                    Message::Text(_) | Message::Binary(_) => {
                        socket
                            .send(Message::Close(Some(CloseFrame {
                                code: 400,
                                reason: "no".into(),
                            })))
                            .await
                    }
                    Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await,
                    Message::Pong(_) => Ok(()),
                    Message::Close(_) => socket.send(Message::Close(None)).await,
                }) else {
                    return;
                };
            }

            res = state.state.changed() => {
                if res.is_err() {
                    return;
                }

                let current_state = state.state.borrow_and_update().clone();
                if !send_state(&mut socket, &current_state).await {
                    return;
                }
            }
        }
    }
}

async fn inputs(State(state): State<AppState>) -> impl IntoResponse {
    #[derive(Serialize)]
    struct Inputs {
        inputs: HashMap<String, InputConfig>,
    }
    Json(Inputs {
        inputs: state.config.inputs,
    })
}

async fn reset(Path(input): Path<String>, State(state): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    if state.config.inputs.contains_key(&input) {
        if let Err(e) = state.command_tx.send(Command::Reset { input }) {
            tracing::error!(?e, "failed to send command");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        } else {
            Ok(Json(serde_json::json!({
                "ok": true
            })))
        }
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

#[cfg(not(debug_assertions))]
async fn index() -> impl IntoResponse {
    Html(include_str!("../web/index.html"))
}

#[cfg(debug_assertions)]
async fn index() -> impl IntoResponse {
    Html(
        tokio::fs::read_to_string("web/index.html")
            .await
            .unwrap(),
    )
}

#[tokio::main]
pub async fn run(
    config: Config,
    state: Receiver<InputsLoudness>,
    command_tx: Sender<Command>,
) -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr = format!("{}:{}", &config.web.host, config.web.port);
    let state = AppState {
        config,
        state,
        command_tx,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/inputs", get(inputs))
        .route("/api/ws", any(events))
        .route("/api/input/{input}/reset", post(reset))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
