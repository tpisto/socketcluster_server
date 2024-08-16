//! Data models for the WebSocket server.
//!
//! This module defines the core data structures used in the WebSocket communication,
//! including the `Event` enum and the `Packet` struct.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Represents different types of events in the WebSocket communication.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Event {
    /// Handshake event for establishing a connection.
    #[serde(rename = "#handshake")]
    Handshake,
    /// Subscribe event for joining a channel.
    #[serde(rename = "#subscribe")]
    Subscribe,
    /// Publish event for sending a message to a channel.
    #[serde(rename = "#publish")]
    Publish,
    /// Unsubscribe event for leaving a channel.
    #[serde(rename = "#unsubscribe")]
    Unsubscribe,
    /// Authenticate event for user authentication. 
    /// !NOTE: Not used in the current implementation
    #[serde(rename = "#authenticate")]
    Authenticate,
    /// Disconnect event for closing the connection.
    #[serde(rename = "#disconnect")]
    Disconnect,
    /// Custom event for application-specific purposes.
    Custom(String),
}

/// Represents a packet of data exchanged in the WebSocket communication.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Packet {
    /// The type of event associated with this packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    /// The main payload of the packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Client-generated ID for the packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<u64>,
    /// Server-generated ID for the response packet. Should match the `cid` of the request packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rid: Option<u64>,
    /// Error information, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}
