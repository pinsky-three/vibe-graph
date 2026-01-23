//! WebSocket handler for real-time updates.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
};
use tokio::sync::broadcast;
use tracing::{debug, error, warn};

use crate::types::{ApiState, WsClientMessage, WsServerMessage};

/// Handler for WebSocket upgrade at GET /api/ws
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<ApiState>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(mut socket: WebSocket, state: Arc<ApiState>) {
    debug!("WebSocket client connected");

    // Subscribe to broadcast channel
    let mut rx = state.tx.subscribe();

    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_client_message(&text, &mut socket).await {
                            warn!("Error handling client message: {}", e);
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(_)) => {
                        // Ignore binary, ping, pong frames
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended");
                        break;
                    }
                }
            }

            // Handle broadcast messages to send to client
            msg = rx.recv() => {
                match msg {
                    Ok(server_msg) => {
                        if let Err(e) = send_server_message(&mut socket, &server_msg).await {
                            error!("Failed to send WebSocket message: {}", e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket client lagged, missed {} messages", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("Broadcast channel closed");
                        break;
                    }
                }
            }
        }
    }
}

/// Handle a message from the client.
async fn handle_client_message(
    text: &str,
    socket: &mut WebSocket,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let msg: WsClientMessage = serde_json::from_str(text)?;

    match msg {
        WsClientMessage::Ping => {
            send_server_message(socket, &WsServerMessage::Pong).await?;
        }
        WsClientMessage::Subscribe { topics } => {
            debug!("Client subscribed to topics: {:?}", topics);
            // For now, all clients receive all messages
            // Topic filtering can be added later
        }
    }

    Ok(())
}

/// Send a server message to the client.
async fn send_server_message(
    socket: &mut WebSocket,
    msg: &WsServerMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let json = serde_json::to_string(msg)?;
    socket.send(Message::Text(json.into())).await?;
    Ok(())
}
