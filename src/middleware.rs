//! Middleware trait for packet processing.
//!
//! This module defines the `Middleware` trait, which allows for custom
//! processing of packets before they are handled by the main application logic.

use crate::models::Packet;
use crate::state::AuthData;
use async_trait::async_trait;

/// Trait for implementing middleware functionality.
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Processes a packet before it's handled by the main application logic.
    ///
    /// # Arguments
    ///
    /// * `packet` - A mutable reference to the `Packet` being processed.
    /// * `auth_data` - A reference to the `AuthData` associated with the connection.
    ///
    /// # Returns
    ///
    /// Returns `true` if the packet should be processed further, `false` if it should be dropped.
    async fn handle(&self, packet: &mut Packet, auth_data: &AuthData) -> bool;
}
