//! Application state management module.
//!
//! This module defines the core state structures for managing WebSocket connections,
//! subscriptions, and middleware in the application.

use crate::config::ServerConfig;
use crate::middleware::Middleware;
use crate::models::Packet;
use crate::Sender;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex as TokioMutex, RwLock};

/// Type alias for socket identifiers.
pub type SocketId = String;

/// Type alias for channel names.
pub type Channel = String;

/// Holds authentication-related data for a socket connection.
pub struct AuthData {
    /// Indicates whether the socket is authenticated.
    pub is_authenticated: std::sync::atomic::AtomicBool,
    /// Authentication token, if any.
    pub token: TokioMutex<Option<String>>,
    /// User identifier, if authenticated.
    pub user_id: TokioMutex<Option<String>>,
}

/// Holds data associated with a socket connection.
pub struct SocketData<S: Sender> {
    /// The sender half of the WebSocket connection.
    pub sender: TokioMutex<S>,
    /// Authentication data for the socket.
    pub auth_data: AuthData,
    /// Timestamp of the last ping received from this socket.
    pub last_ping: TokioMutex<std::time::Instant>,
}

/// Represents the global application state.
pub struct AppState<S: Sender> {
    /// Map of all active socket connections.
    pub sockets: Arc<RwLock<HashMap<SocketId, SocketData<S>>>>,
    /// Map of channel subscriptions.
    pub subscriptions: Arc<RwLock<HashMap<Channel, HashSet<SocketId>>>>,
    /// Server configuration.
    pub config: ServerConfig,
    /// List of middleware to be applied to incoming packets.
    middleware: Arc<Vec<Arc<dyn Middleware>>>,
}

impl<S: Sender> AppState<S> {
    /// Creates a new `AppState` instance with the given configuration.
    pub fn new(config: ServerConfig) -> Self {
        AppState {
            sockets: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            config,
            middleware: Arc::new(vec![]),
        }
    }

    /// Adds a new middleware to the application state.
    ///
    /// Middleware added using this method will be applied in the order they are added to all incoming
    /// packets. Middleware can inspect, modify, or reject packets based on their logic.
    ///
    /// # Arguments
    ///
    /// * `middleware` - An `Arc` wrapped instance of a type that implements the `Middleware` trait.
    pub fn add_middleware(&mut self, middleware: Arc<dyn Middleware>) {
        Arc::get_mut(&mut self.middleware).unwrap().push(middleware);
    }

    /// Applies all registered middleware to an incoming packet.
    ///
    /// This method iterates through all middleware in the order they were added and applies them to
    /// the provided packet. If any middleware returns `false`, the processing stops, and the packet
    /// is rejected.
    ///
    /// # Arguments
    ///
    /// * `packet` - A mutable reference to the `Packet` to which the middleware will be applied.
    /// * `socket_id` - The `SocketId` of the connection associated with the packet.
    ///
    /// # Returns
    ///
    /// A boolean indicating whether the packet passed all middleware checks (`true` if passed,
    /// `false` if rejected).
    pub(crate) async fn apply_middleware(&self, packet: &mut Packet, socket_id: &SocketId) -> bool {
        let sockets = self.sockets.read().await;
        if let Some(socket_data) = sockets.get(socket_id) {
            for middleware in self.middleware.iter() {
                if !middleware.handle(packet, &socket_data.auth_data).await {
                    return false;
                }
            }
        }
        true
    }
}

impl<S: Sender> Clone for AppState<S> {
    /// Creates a clone of the `AppState`.
    ///
    /// This implementation performs a shallow clone of the internal structures,
    /// creating new `Arc` pointers to the same underlying data.
    ///
    /// # Returns
    ///
    /// A new `AppState` instance with shared internal state.
    fn clone(&self) -> Self {
        AppState {
            sockets: self.sockets.clone(),
            subscriptions: self.subscriptions.clone(),
            config: self.config.clone(),
            middleware: self.middleware.clone(),
        }
    }

    /// Replaces the contents of `self` with a clone of `source`.
    ///
    /// This method is an optimization that reuses the existing allocation
    /// when possible.
    ///
    /// # Arguments
    ///
    /// * `source` - The `AppState` to clone from.
    fn clone_from(&mut self, source: &Self) {
        *self = source.clone()
    }
}
