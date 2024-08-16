//! Configuration module for the WebSocket server.
//!
//! This module defines the `ServerConfig` struct and provides functionality
//! to load the configuration from a source (e.g., file, environment variables).

use serde::Deserialize;
use std::time::Duration;

/// Server configuration struct.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Interval for sending ping messages to clients.
    #[serde(deserialize_with = "crate::utils::deserialize_duration")]
    pub ping_interval: Duration,

    /// Timeout duration for considering a client disconnected if no pong is received.
    #[serde(deserialize_with = "crate::utils::deserialize_duration")]
    pub ping_timeout: Duration,

    /// Port number on which the server will listen.
    pub port: u16,

    /// Host address on which the server will bind.
    pub host: String,

    /// Secret key for JWT authentication.
    pub jwt_secret: String,
}
