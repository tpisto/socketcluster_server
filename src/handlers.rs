//! WebSocket connection handlers and related traits.
//!
//! This module contains the main WebSocket handler and associated traits for
//! sending and receiving WebSocket messages.

use crate::config::ServerConfig;
use crate::models::{Event, Packet};
use crate::state::{AppState, AuthData, SocketData, SocketId};
use async_trait::async_trait;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::{extract::State, response::IntoResponse};
use axum_extra::TypedHeader;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Trait for sending WebSocket messages.
#[async_trait]
pub trait Sender: Send + Sync {
    /// Sends a WebSocket message.
    ///
    /// # Arguments
    ///
    /// * `message` - The message to send.
    ///
    /// # Errors
    ///
    /// Returns an `axum::Error` if sending fails.
    async fn send(&mut self, message: AxumMessage) -> Result<(), axum::Error>;
}

/// Trait for receiving WebSocket messages.
#[async_trait]
pub trait Receiver: Send + Sync {
    /// Receives the next WebSocket message.
    ///
    /// # Returns
    ///
    /// Returns `Some(Result<AxumMessage, axum::Error>)` if a message is received,
    /// or `None` if the connection is closed.
    async fn next(&mut self) -> Option<Result<AxumMessage, axum::Error>>;
}

/// Implements the `Sender` trait for the WebSocket sink.
pub struct WebSocketSender(futures::stream::SplitSink<WebSocket, AxumMessage>);

/// Implements the `Receiver` trait for the WebSocket stream.
pub struct WebSocketReceiver(futures::stream::SplitStream<WebSocket>);

#[async_trait]
impl Sender for WebSocketSender {
    async fn send(&mut self, message: AxumMessage) -> Result<(), axum::Error> {
        debug!("Sending message: {:?}", message);
        self.0.send(message).await
    }
}

#[async_trait]
impl Receiver for WebSocketReceiver {
    async fn next(&mut self) -> Option<Result<AxumMessage, axum::Error>> {
        let result = self.0.next().await;
        debug!("Received message: {:?}", result);
        result
    }
}

/// Handles incoming WebSocket connection requests.
///
/// # Arguments
///
/// * `ws` - WebSocket upgrade.
/// * `user_agent` - Optional user agent header.
/// * `addr` - Client's socket address.
/// * `state` - Application state.
///
/// # Returns
///
/// Returns an `impl IntoResponse` which upgrades the connection to a WebSocket.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState<WebSocketSender>>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    info!("New WebSocket connection: `{user_agent}` at {addr}");
    ws.on_upgrade(move |socket| handle_websocket_socket(socket, state))
}

async fn handle_websocket_socket(socket: WebSocket, state: AppState<WebSocketSender>) {
    let mut socket_id = Uuid::new_v4().to_string();
    // Check that socket_id does not exist, if it does, generate a new one
    while state.sockets.read().await.contains_key(&socket_id) {
        debug!("Socket ID collision, generating new ID");
        socket_id = Uuid::new_v4().to_string();
    }
    debug!("New socket connected with ID: {}", socket_id);

    let (sender, receiver) = socket.split();

    let socket_data = SocketData {
        sender: TokioMutex::new(WebSocketSender(sender)),
        auth_data: AuthData {
            is_authenticated: AtomicBool::new(false),
            token: TokioMutex::new(None),
            user_id: TokioMutex::new(None),
        },
        last_ping: TokioMutex::new(std::time::Instant::now()),
    };

    {
        state
            .sockets
            .write()
            .await
            .insert(socket_id.clone(), socket_data);
        debug!("Socket {} added to state", socket_id);
    }
    handle_socket(socket_id, WebSocketReceiver(receiver), state).await;
}

pub(crate) async fn handle_socket<S: Sender + 'static, R: Receiver>(
    socket_id: String,
    mut receiver: R,
    state: AppState<S>,
) {
    debug!("Starting to handle socket: {}", socket_id);

    // Wait for the handshake from the client
    let handshake = match receiver.next().await {
        Some(Ok(AxumMessage::Text(text))) => {
            debug!("Received handshake text from {}: {}", socket_id, text);
            serde_json::from_str::<Packet>(&text).ok()
        }
        _ => {
            warn!("Invalid handshake received from {}", socket_id);
            None
        }
    };

    if let Some(mut packet) = handshake {
        if packet.event != Some(Event::Handshake)
            || !state.apply_middleware(&mut packet, &socket_id).await
        {
            warn!("Handshake failed for {}, disconnecting", socket_id);
            state.sockets.write().await.remove(&socket_id);
            return;
        }
        handle_handshake(socket_id.clone(), packet, &state).await;
    } else {
        warn!(
            "No valid handshake received from {}, disconnecting",
            socket_id
        );
        state.sockets.write().await.remove(&socket_id);
        return;
    }

    // Start ping interval
    let mut ping_interval = interval(state.config.ping_interval);
    let ping_state = state.clone();
    let ping_socket_id = socket_id.clone();
    tokio::spawn(async move {
        debug!("Starting ping interval for {}", ping_socket_id);
        ping_interval.tick().await;
        ping_socket(ping_socket_id, ping_interval, ping_state).await;
    });

    // Main message loop
    debug!("Entering main message loop for {}", socket_id);
    while let Some(msg) = receiver.next().await {
        if let Ok(msg) = msg {
            match msg {
                // Socketcluster V1 ping message
                AxumMessage::Text(text) if text == "#2" => {
                    debug!("Received pong from {}", socket_id);
                    handle_pong(socket_id.clone(), &state).await
                }
                // Handle all the actions
                AxumMessage::Text(text) => {
                    debug!("Received text message from {}: {}", socket_id, text);
                    if let Ok(mut packet) = serde_json::from_str::<Packet>(&text) {
                        if state.apply_middleware(&mut packet, &socket_id).await {
                            handle_packet(socket_id.clone(), packet, &state).await;
                        } else {
                            warn!("Middleware rejected packet from {}", socket_id);
                        }
                    } else {
                        warn!("Failed to parse packet from {}: {}", socket_id, text);
                    }
                }
                // Handle websocket ping messages
                AxumMessage::Ping(payload) => {
                    debug!("Received WebSocket ping from {}", socket_id);
                    if let Err(e) = handle_ws_ping(socket_id.clone(), payload, &state).await {
                        error!("Failed to send pong to {}: {:?}", socket_id, e);
                        break;
                    }
                }
                AxumMessage::Close(frame) => {
                    info!("Received close frame from {}: {:?}", socket_id, frame);
                    break;
                }
                _ => {
                    debug!("Received other message type from {}: {:?}", socket_id, msg);
                }
            }
        } else {
            error!("Error receiving message from {}: {:?}", socket_id, msg);
            break;
        }
    }

    debug!("Exited main message loop for {}", socket_id);
    // Clean up on disconnect
    handle_disconnect(socket_id, &state).await;
}

async fn handle_ws_ping<S: Sender>(
    socket_id: SocketId,
    payload: Vec<u8>,
    state: &AppState<S>,
) -> Result<(), axum::Error> {
    debug!("Handling WebSocket ping for {}", socket_id);
    let sockets = state.sockets.read().await;
    if let Some(socket_data) = sockets.get(&socket_id) {
        let mut sender = socket_data.sender.lock().await;
        sender.send(AxumMessage::Pong(payload)).await?;
        debug!("Sent WebSocket pong to {}", socket_id);
    } else {
        warn!("Socket {} not found for WebSocket ping", socket_id);
    }
    Ok(())
}

async fn handle_handshake<S: Sender>(socket_id: SocketId, packet: Packet, state: &AppState<S>) {
    debug!("Handling handshake for {}", socket_id);
    if let Some(socket_data) = state.sockets.read().await.get(&socket_id) {
        // In V1, we only send a response if cid is present
        if packet.cid.is_some() {
            let handshake_response = Packet {
                rid: packet.cid,
                data: Some(json!({
                    "id": socket_id,
                    "pingTimeout": state.config.ping_timeout.as_millis(),
                    // Authentication should be handled by user own middleware, we don't provide any default authentication methods
                    "isAuthenticated": socket_data.auth_data.is_authenticated.load(std::sync::atomic::Ordering::Relaxed),
                })),
                ..Default::default()
            };

            let mut sender = socket_data.sender.lock().await;
            match sender
                .send(AxumMessage::Text(
                    serde_json::to_string(&handshake_response).unwrap(),
                ))
                .await
            {
                Ok(_) => debug!("Sent handshake response to {}", socket_id),
                Err(e) => error!(
                    "Failed to send handshake response to {}: {:?}",
                    socket_id, e
                ),
            }
        } else {
            debug!(
                "No cid in handshake packet from {}, not sending response",
                socket_id
            );
        }
    } else {
        warn!("Socket {} not found for handshake", socket_id);
    }
}

async fn ping_socket<S: Sender>(
    socket_id: SocketId,
    mut interval: tokio::time::Interval,
    state: AppState<S>,
) {
    debug!("Starting ping cycle for {}", socket_id);
    loop {
        interval.tick().await;
        debug!("Ping interval elapsed for {}", socket_id);
        let mut should_disconnect = false;
        {
            let sockets = state.sockets.read().await;
            if let Some(socket_data) = sockets.get(&socket_id) {
                if socket_data.last_ping.lock().await.elapsed() > state.config.ping_timeout {
                    warn!("Ping timeout for {}", socket_id);
                    should_disconnect = true;
                } else {
                    let mut sender = socket_data.sender.lock().await;
                    match sender.send(AxumMessage::Text("#1".to_string())).await {
                        Ok(_) => debug!("Sent ping to {}", socket_id),
                        Err(e) => {
                            error!("Failed to send ping to {}: {:?}", socket_id, e);
                            should_disconnect = true;
                        }
                    }
                }
            } else {
                warn!("Socket {} not found for ping", socket_id);
                break;
            }
        }
        if should_disconnect {
            warn!("Disconnecting {} due to ping issues", socket_id);
            handle_disconnect(socket_id.clone(), &state).await;
            break;
        }
    }
    debug!("Ended ping cycle for {}", socket_id);
}

async fn handle_pong<S: Sender>(socket_id: SocketId, state: &AppState<S>) {
    debug!("Handling pong from {}", socket_id);
    let sockets = state.sockets.read().await;
    if let Some(socket_data) = sockets.get(&socket_id) {
        *socket_data.last_ping.lock().await = std::time::Instant::now();
        debug!("Updated last ping time for {}", socket_id);
    } else {
        warn!("Socket {} not found for pong", socket_id);
    }
}

async fn handle_packet<S: Sender>(socket_id: SocketId, packet: Packet, state: &AppState<S>) {
    debug!("Handling packet from {}: {:?}", socket_id, packet);
    if let Some(event) = &packet.event {
        match event {
            Event::Subscribe => handle_subscribe(socket_id, packet, state).await,
            Event::Publish => handle_publish(socket_id, packet, state).await,
            Event::Unsubscribe => handle_unsubscribe(socket_id, packet, state).await,
            Event::Custom(_) => handle_custom_event(socket_id, packet).await,
            // For now, let's not support authentication event and send auth token instead in handshake
            // Event::Authenticate => handle_authenticate(socket_id, packet, state).await,
            _ => debug!("Unhandled event type for {}: {:?}", socket_id, event),
        }
    } else {
        warn!("Received packet without event from {}", socket_id);
    }
}

async fn handle_subscribe<S: Sender>(socket_id: SocketId, packet: Packet, state: &AppState<S>) {
    debug!("Handling subscribe for {}: {:?}", socket_id, packet);
    if let Some(Value::String(channel)) = packet.data.as_ref().and_then(|d| d.get("channel")) {
        let mut subscriptions = state.subscriptions.write().await;
        subscriptions
            .entry(channel.clone())
            .or_insert_with(HashSet::new)
            .insert(socket_id.clone());
        debug!("Added {} to channel {}", socket_id, channel);

        let confirmation = Packet {
            rid: packet.cid,
            ..Default::default()
        };

        let sockets = state.sockets.read().await;
        if let Some(socket_data) = sockets.get(&socket_id) {
            let mut sender = socket_data.sender.lock().await;
            match sender
                .send(AxumMessage::Text(
                    serde_json::to_string(&confirmation).unwrap(),
                ))
                .await
            {
                Ok(_) => debug!("Sent subscribe confirmation to {}", socket_id),
                Err(e) => error!(
                    "Failed to send subscribe confirmation to {}: {:?}",
                    socket_id, e
                ),
            }
        } else {
            warn!("Socket {} not found for subscribe confirmation", socket_id);
        }
    } else {
        warn!("Invalid subscribe packet from {}: {:?}", socket_id, packet);
    }
}

async fn handle_publish<S: Sender>(socket_id: SocketId, packet: Packet, state: &AppState<S>) {
    debug!("Handling publish for {}: {:?}", socket_id, packet);
    if let Some(Value::Object(data)) = packet.data {
        if let (Some(Value::String(channel)), Some(message)) =
            (data.get("channel"), data.get("data"))
        {
            let subscribers = {
                let subscriptions = state.subscriptions.read().await;
                subscriptions.get(channel).cloned()
            };
            if let Some(subscribers) = subscribers {
                let publish_event = Packet {
                    event: Some(Event::Publish),
                    data: Some(message.clone()),
                    ..Default::default()
                };

                let sockets = state.sockets.read().await;
                for sub_socket_id in subscribers {
                    if let Some(socket_data) = sockets.get(&sub_socket_id) {
                        let mut sender = socket_data.sender.lock().await;
                        match sender
                            .send(AxumMessage::Text(
                                serde_json::to_string(&publish_event).unwrap(),
                            ))
                            .await
                        {
                            Ok(_) => debug!("Sent publish event to {}", sub_socket_id),
                            Err(e) => {
                                error!("Failed to send publish event to {}: {:?}", sub_socket_id, e)
                            }
                        }
                    } else {
                        warn!(
                            "Subscriber {} not found for channel {}",
                            sub_socket_id, channel
                        );
                    }
                }
            }
            debug!("Published message to channel {}", channel);

            if packet.cid.is_some() {
                let response = Packet {
                    rid: packet.cid,
                    ..Default::default()
                };
                let sockets = state.sockets.read().await;
                if let Some(socket_data) = sockets.get(&socket_id) {
                    let mut sender = socket_data.sender.lock().await;
                    match sender
                        .send(AxumMessage::Text(serde_json::to_string(&response).unwrap()))
                        .await
                    {
                        Ok(_) => debug!("Sent publish confirmation to {}", socket_id),
                        Err(e) => error!(
                            "Failed to send publish confirmation to {}: {:?}",
                            socket_id, e
                        ),
                    }
                } else {
                    warn!("Socket {} not found for publish confirmation", socket_id);
                }
            }
        } else {
            warn!("Invalid publish packet from {}", socket_id);
        }
    } else {
        warn!(
            "Invalid publish packet data from {}: {:?}",
            socket_id, packet
        );
    }
}

async fn handle_unsubscribe<S: Sender>(socket_id: SocketId, packet: Packet, state: &AppState<S>) {
    debug!("Handling unsubscribe for {}: {:?}", socket_id, packet);
    if let Some(Value::String(channel)) = packet.data {
        let mut subscriptions = state.subscriptions.write().await;
        if let Some(subscribers) = subscriptions.get_mut(&channel) {
            subscribers.remove(&socket_id);
            debug!("Removed {} from channel {}", socket_id, channel);
        } else {
            warn!(
                "Channel {} not found for unsubscribe from {}",
                channel, socket_id
            );
        }
        drop(subscriptions);

        if packet.cid.is_some() {
            let response = Packet {
                rid: packet.cid,
                ..Default::default()
            };
            if let Some(socket_data) = state.sockets.read().await.get(&socket_id) {
                let mut sender = socket_data.sender.lock().await;
                match sender
                    .send(AxumMessage::Text(serde_json::to_string(&response).unwrap()))
                    .await
                {
                    Ok(_) => debug!("Sent unsubscribe confirmation to {}", socket_id),
                    Err(e) => error!(
                        "Failed to send unsubscribe confirmation to {}: {:?}",
                        socket_id, e
                    ),
                }
            } else {
                warn!(
                    "Socket {} not found for unsubscribe confirmation",
                    socket_id
                );
            }
        }
    } else {
        warn!(
            "Invalid unsubscribe packet from {}: {:?}",
            socket_id, packet
        );
    }
}

async fn handle_custom_event(socket_id: SocketId, packet: Packet) {
    debug!(
        "Handling custom event from {}: {:?} with data: {:?}",
        socket_id, packet.event, packet.data
    );
    // Custom event handling logic can be added here
}

async fn handle_disconnect<S: Sender>(socket_id: SocketId, state: &AppState<S>) {
    debug!("Handling disconnect for {}", socket_id);
    if state.sockets.write().await.remove(&socket_id).is_some() {
        debug!("Removed socket {} from state", socket_id);
    } else {
        warn!("Socket {} not found in state for disconnect", socket_id);
    }

    let mut subscriptions = state.subscriptions.write().await;
    for (channel, subscribers) in subscriptions.iter_mut() {
        if subscribers.remove(&socket_id) {
            debug!("Removed {} from channel {}", socket_id, channel);
        }
    }
    drop(subscriptions);

    let sockets = state.sockets.read().await;
    if let Some(socket_data) = sockets.get(&socket_id) {
        let disconnect_event = Packet {
            event: Some(Event::Disconnect),
            ..Default::default()
        };
        let mut sender = socket_data.sender.lock().await;
        match sender
            .send(AxumMessage::Text(
                serde_json::to_string(&disconnect_event).unwrap(),
            ))
            .await
        {
            Ok(_) => debug!("Sent disconnect event to {}", socket_id),
            Err(e) => error!("Failed to send disconnect event to {}: {:?}", socket_id, e),
        }
    } else {
        warn!(
            "Socket {} not found for sending disconnect event",
            socket_id
        );
    }
    info!("Completed disconnect process for {}", socket_id);
}

/// Creates a new `AppState` instance for the SocketCluster server.
///
/// This function is a convenience wrapper around `AppState::new()` that creates
/// a new application state with the provided configuration.
///
/// # Arguments
///
/// * `config` - The `ServerConfig` containing the configuration settings for the server.
///
/// # Type Parameters
///
/// * `S` - The type implementing the `Sender` trait, which is used for sending WebSocket messages.
///
/// # Returns
///
/// Returns a new `AppState<S>` instance initialized with the given configuration.
pub fn create_socketcluster_state<S: Sender>(config: ServerConfig) -> AppState<S> {
    debug!("Creating new SocketCluster state with config: {:?}", config);
    AppState::new(config)
}
