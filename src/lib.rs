//! # Rust SocketCluster-Inspired WebSocket Server
//!
//! This library provides scalable SocketCluster protocol v1 server implementation.
//! It's built on top of Axum and Tokio, offering a robust foundation for real-time applications.
//!
//! ## Features
//!
//! - WebSocket-based real-time communication
//! - Publish/Subscribe pattern for efficient message distribution
//! - Customizable middleware for packet processing
//! - Authentication support
//! - Ping/Pong mechanism for connection health monitoring
//!
//! ## Main Components
//!
//! - `AppState`: Manages the global state of the application, including active connections and subscriptions.
//! - `Handlers`: Processes WebSocket events and messages.
//! - `Middleware`: Allows for custom processing of packets before they reach the main application logic.
//! - `Models`: Defines the core data structures used in communication.
//! - `Config`: Handles server configuration.
//!
//! ## Getting Started
//!
//! To use this library, add it to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! rust_socketcluster = "0.1.0"
//! ```
//!
//! Then, you can start using it in your project. Here's a basic example:
//!
//! ```no_run
//! use socketcluster_server::{create_socketcluster_state, ServerConfig, ws_handler};
//! use axum::{Router, routing::get};
//! use tokio::net::TcpListener;
//! use std::net::SocketAddr;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Load configuration
//!     let config = ServerConfig {
//!         ping_interval: std::time::Duration::from_secs(30),
//!         ping_timeout: std::time::Duration::from_secs(5),
//!         port: 8080,
//!         host: "127.0.0.1".to_string(),
//!         jwt_secret: "your-secret-key".to_string(),
//!     };
//!
//!     // Create application state
//!     let state = create_socketcluster_state(config.clone());
//!
//!     // Set up router
//!     let app = Router::new()
//!         .route("/ws", get(ws_handler))
//!         .with_state(state);
//!
//!     // Start the server
//!     let addr = format!("{}:{}", config.host, config.port);
//!     let listener = TcpListener::bind(&addr).await.unwrap();
//!     println!("Server listening on: {}", addr);
//!     axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
//! }
//! ```
//!
//! ## Advanced Usage
//!
//! ### Custom Middleware
//!
//! You can add custom middleware to process packets before they reach the main application logic:
//!
//! ```rust
//! use socketcluster_server::{Middleware, Packet, AuthData};
//! use async_trait::async_trait;
//!
//! struct MyMiddleware;
//!
//! #[async_trait]
//! impl Middleware for MyMiddleware {
//!     async fn handle(&self, packet: &mut Packet, auth_data: &AuthData) -> bool {
//!         // Custom packet processing logic
//!         println!("Processing packet: {:?}", packet);
//!         true // Allow the packet to proceed
//!     }
//! }
//!
//! // In your main function:
//! let mut state = create_socketcluster_state(config);
//! state.add_middleware(Arc::new(MyMiddleware));
//! ```
//!
//! ## Best Practices
//!
//! - Use the `AppState` to manage shared resources and avoid race conditions.
//! - Implement proper error handling and logging in your application.
//! - Use SSL/TLS for secure WebSocket connections in production (for example by Haproxy SSL termination).
//!
//! ## Contributing
//!
//! Contributions are welcome! Please feel free to submit a Pull Request.

mod config;
mod handlers;
mod middleware;
mod models;
mod state;
mod utils;

pub use config::*;
pub use handlers::*;
pub use middleware::*;
pub use models::*;
pub use state::*;

#[cfg(test)]
#[path = "tests/test_messages.rs"]
mod test_messages;
