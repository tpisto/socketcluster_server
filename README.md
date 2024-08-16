[<img alt="github" src="https://img.shields.io/badge/github-tpisto/socketcluster_server-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/tpisto/socketcluster_server)
[<img alt="crates.io" src="https://img.shields.io/crates/v/socketcluster_server.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/socketcluster_server)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-socketcluster_server-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/socketcluster_server)
[<img alt="tests" src="https://img.shields.io/github/actions/workflow/status/tpisto/socketcluster_server/rust.yml?branch=main&style=for-the-badge" height="20">](https://github.com/tpisto/socketcluster_server/actions?query=branch%3Amain)

<img src="https://github.com/user-attachments/assets/c0137936-ba13-4c9b-8324-8c454512ca3d" width="100px">

# Rust SocketCluster protocol V1 server library

A scalable SocketCluster WebSocket server implementation as a library, built with Rust using Axum and Tokio.

## Features

- 🚀 WebSocket-based real-time communication
- 📡 Publish/Subscribe pattern for efficient message distribution
- 🔌 Customizable middleware for packet processing
- 🔐 JWT Authentication support
- 💓 Ping/Pong mechanism for connection health monitoring
- 🌐 HTTP endpoints integration alongside WebSocket functionality
- ⚙️ Flexible configuration using TOML files

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Usage Examples](#usage-examples)
  - [Setting up the Server](#setting-up-the-server)
  - [Implementing Custom Middleware](#implementing-custom-middleware)
  - [HTTP Endpoint for Publishing Messages directly](#http-endpoint-for-publishing-messages-directly)
- [API Documentation](#api-documentation)
- [Contributing](#contributing)
- [License](#license)

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
socketcluster_server = "0.1.0"
```

## Quick Start

Here's a minimal example to get a server up and running:

```rust
use socketcluster_server::{create_socketcluster_state, ServerConfig, ws_handler};
use axum::{Router, routing::get};
use tokio::net::TcpListener;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    // Load configuration
    let config = ServerConfig {
        ping_interval: std::time::Duration::from_secs(30),
        ping_timeout: std::time::Duration::from_secs(5),
        port: 8080,
        host: "127.0.0.1".to_string(),
        jwt_secret: "your-secret-key".to_string(),
    };

    // Create application state
    let state = create_socketcluster_state(config.clone());

    // Set up router
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    // Start the server
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    println!("Server listening on: {}", addr);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}
```

## Configuration

The server can be configured using a TOML file. Create a `config/settings.toml` file with the following structure:

```toml
ping_interval = 30  # in seconds
ping_timeout = 5    # in seconds
port = 8080
host = "127.0.0.1"
jwt_secret = "your-secret-key"
```

## Usage Examples

### HTTP Endpoint for Publishing Messages directly

This server is a library, so you can use it directly with Axum. Here you can
find an simple example how you can directly operate the socketcluster channels and 
publish, just using Axum route endpoints.

```rust
// ... use ...

// Axum HTTP handler to send messages directly to the channels
async fn send_handler<S: Sender>(
    State(state): State<AppState<S>>,
    TypedHeader(auth_header): TypedHeader<headers::Authorization<headers::authorization::Bearer>>,
    Json(payload): Json<serde_json::value::Map<String, Value>>,
) -> impl IntoResponse {

    // JWT validation logic here...

    let channel = payload.get("channel").unwrap().as_str().unwrap();
    let message = payload.get("message").unwrap();

    if let Some(subscribers) = state.subscriptions.read().await.get(channel) {
        let publish_event = Packet {
            event: Some(Event::Publish),
            data: Some(message.clone()),
            ..Default::default()
        };

        for sub_socket_id in subscribers {
            if let Some(socket_data) = state.sockets.read().await.get(sub_socket_id) {
                let mut sender = socket_data.sender.lock().await;
                let _ = sender.send(AxumMessage::Text(serde_json::to_string(&publish_event).unwrap())).await;
            }
        }
        (StatusCode::OK, json!({ "ok": true }).to_string()).into_response()
    } else {
        (StatusCode::NOT_FOUND, json!({ "ok": false, "message": "channel not found" }).to_string()).into_response()
    }
}

#[tokio::main]
async fn main() {
  let settings = config::Config::builder()
      .add_source(config::File::with_name("config/settings.toml"))
      .build()
      .expect("Configuration loading failed");

  let config: ServerConfig = settings.clone().try_deserialize().expect("Failed to deserialize configuration");

  let mut sc_state = create_socketcluster_state::<WebSocketSender>(config.clone());

  // Add routes and start the server
  let app = Router::new()
      // Main socketcluster client handler (websocket connection)
      .route("/ws/", get(ws_handler))
      // Axum POST handler to allow sending messages to channels directly
      .route("/publish_to_channel", post(send_handler))
      .with_state(sc_state);

  let addr = format!("{}:{}", config.host, config.port);
  let listener = TcpListener::bind(&addr).await.unwrap();
  axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}
```

### Implementing Custom Middleware

```rust
struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, packet: &mut Packet, _auth_data: &AuthData) -> bool {
        println!("Processing packet: {:?}", packet);
        true // Allow the packet to proceed
    }
}

// In your main function:
let logging_middleware = Arc::new(LoggingMiddleware);
sc_state.add_middleware(logging_middleware);
```

## API Documentation

For detailed API documentation, run:

```
cargo doc --open
```
or go to https://docs.rs/fuzzy_prefix_search/latest/fuzzy_prefix_search/

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
