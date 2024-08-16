#[cfg(test)]
mod tests {
    use crate::{
        create_socketcluster_state, handle_socket, AuthData, Receiver as SCServerReceiver,
        Sender as SCServerSender, ServerConfig, SocketData,
    };
    use async_trait::async_trait;
    use axum::extract::ws::Message as AxumMessage;
    use axum::Error;
    use futures::future::join_all;
    use std::collections::HashMap;
    use std::sync::{atomic::AtomicBool, Arc};
    use std::time::Duration;
    use tokio::sync::{mpsc, Mutex as TokioMutex};

    #[derive(Debug, Clone, PartialEq)]
    enum TestMessage {
        Send(String),
        Expect(String),
    }

    #[derive(Clone)]
    struct MockSocket {
        incoming: Arc<TokioMutex<mpsc::UnboundedReceiver<TestMessage>>>,
        outgoing: mpsc::UnboundedSender<TestMessage>,
    }

    #[async_trait]
    impl SCServerSender for MockSocket {
        async fn send(&mut self, message: AxumMessage) -> Result<(), Error> {
            // println!("Socket {} sending message: {:?}", self.id, message);
            if let AxumMessage::Text(text) = message {
                self.outgoing
                    .send(TestMessage::Send(text))
                    .map_err(|e| Error::new(e))?;
            }
            Ok(())
        }
    }

    #[async_trait]
    impl SCServerReceiver for MockSocket {
        async fn next(&mut self) -> Option<Result<AxumMessage, Error>> {
            let message = self.incoming.lock().await.recv().await;
            // println!("Socket {} received message: {:?}", self.id, message);
            match message {
                Some(TestMessage::Expect(text)) => Some(Ok(AxumMessage::Text(text))),
                _ => None,
            }
        }
    }

    struct TestClient {
        id: String,
        sender: mpsc::UnboundedSender<TestMessage>,
        receiver: mpsc::UnboundedReceiver<TestMessage>,
    }

    impl TestClient {
        fn new(id: String) -> (Self, MockSocket) {
            let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
            let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();

            let client = TestClient {
                id: id.clone(),
                sender: incoming_tx,
                receiver: outgoing_rx,
            };

            let mock_socket = MockSocket {
                incoming: Arc::new(TokioMutex::new(incoming_rx)),
                outgoing: outgoing_tx,
            };

            (client, mock_socket)
        }

        async fn send(&self, message: String) {
            self.sender.send(TestMessage::Expect(message)).unwrap();
        }

        async fn expect(&mut self, expected: String) {
            match tokio::time::timeout(Duration::from_secs(5), self.receiver.recv()).await {
                Ok(Some(TestMessage::Send(actual))) => {
                    assert_eq!(
                        expected, actual,
                        "Client {} received unexpected message",
                        self.id
                    );
                }
                Ok(Some(TestMessage::Expect(_))) => {
                    panic!("Client {} received unexpected TestMessage::Expect", self.id);
                }
                Ok(None) => panic!("Client {} channel closed unexpectedly", self.id),
                Err(_) => panic!("Timeout waiting for expected message on client {}", self.id),
            }
        }
    }

    async fn run_multi_client_test(client_messages: HashMap<String, Vec<TestMessage>>) {
        let config = ServerConfig {
            ping_interval: Duration::from_millis(1000),
            ping_timeout: Duration::from_secs(10),
            port: 8000,
            host: "localhost".into(),
            jwt_secret: "secret".into(),
        };

        let state = create_socketcluster_state::<MockSocket>(config);

        let mut client_handles = vec![];
        let mut test_client_handles = vec![];

        for (client_id, messages) in client_messages {
            let (test_client, mock_socket) = TestClient::new(client_id.clone());

            let socket_data = SocketData {
                sender: TokioMutex::new(mock_socket.clone()),
                auth_data: AuthData {
                    is_authenticated: AtomicBool::new(false),
                    token: TokioMutex::new(None),
                    user_id: TokioMutex::new(None),
                },
                last_ping: TokioMutex::new(std::time::Instant::now()),
            };
            state
                .sockets
                .write()
                .await
                .insert(client_id.clone(), socket_data);

            let client_state = state.clone();
            let handle = tokio::spawn(async move {
                handle_socket(client_id, mock_socket, client_state).await;
            });
            client_handles.push(handle);

            let test_handle = tokio::spawn(async move {
                process_client_messages(test_client, messages).await;
            });
            test_client_handles.push(test_handle);
        }

        // Wait for all test client handlers to complete
        let results = join_all(test_client_handles).await;

        // Check if any test client handler failed
        for result in results {
            assert!(result.is_ok(), "Test client handler failed");
        }

        // Wait for all socket handlers to complete
        join_all(client_handles).await;

        assert!(
            state.sockets.read().await.is_empty(),
            "Not all sockets were removed from state after disconnection"
        );
    }

    async fn process_client_messages(mut client: TestClient, messages: Vec<TestMessage>) {
        for msg in messages {
            match msg {
                TestMessage::Send(message) => {
                    client.send(message).await;
                }
                TestMessage::Expect(expected) => {
                    client.expect(expected).await;
                }
            }
        }
    }

    #[tokio::test]
    async fn test_handle_socket_handshake_success() {
        let mut client_messages = HashMap::new();

        client_messages.insert(
            "test_socket".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1, \"pingTimeout\":10000}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"test_socket\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                TestMessage::Expect("#1".into()), // Expect ping message
                TestMessage::Send("#2".into()),   // Send pong message
            ],
        );

        run_multi_client_test(client_messages).await;
    }

    #[tokio::test]
    async fn test_multiple_clients_subscribe_and_publish() {
        let mut client_messages = HashMap::new();

        // Client 1: Subscribes and publishes
        client_messages.insert(
            "client1".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"client1\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"test-channel\"},\"cid\":2}".into()),
                TestMessage::Expect("{\"rid\":2}".into()),
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"test-channel\",\"data\":\"Hello from client1\"},\"cid\":3}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Hello from client1\"}".into()),
                TestMessage::Expect("{\"rid\":3}".into()),
            ],
        );

        // Client 2: Subscribes and receives
        client_messages.insert(
            "client2".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"client2\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"test-channel\"},\"cid\":2}".into()),
                TestMessage::Expect("{\"rid\":2}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Hello from client1\"}".into()),
            ],
        );

        run_multi_client_test(client_messages).await;
    }

    #[tokio::test]
    async fn test_unsubscribe() {
        let mut client_messages = HashMap::new();

        client_messages.insert(
            "client1".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"client1\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"test-channel\"},\"cid\":2}".into()),
                TestMessage::Expect("{\"rid\":2}".into()),
                TestMessage::Send("{\"event\":\"#unsubscribe\",\"data\":\"test-channel\",\"cid\":3}".into()),
                TestMessage::Expect("{\"rid\":3}".into()),
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"test-channel\",\"data\":\"Hello\"},\"cid\":4}".into()),
                TestMessage::Expect("{\"rid\":4}".into()),
                // No expect for the published message, as we've unsubscribed
            ],
        );

        run_multi_client_test(client_messages).await;
    }

    #[tokio::test]
    async fn test_multiple_channels() {
        let mut client_messages = HashMap::new();

        client_messages.insert(
            "client1".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"client1\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"channel1\"},\"cid\":2}".into()),
                TestMessage::Expect("{\"rid\":2}".into()),
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"channel2\"},\"cid\":3}".into()),
                TestMessage::Expect("{\"rid\":3}".into()),
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"channel1\",\"data\":\"Hello channel1\"},\"cid\":4}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Hello channel1\"}".into()),
                TestMessage::Expect("{\"rid\":4}".into()),
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"channel2\",\"data\":\"Hello channel2\"},\"cid\":5}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Hello channel2\"}".into()),
                TestMessage::Expect("{\"rid\":5}".into()),
            ],
        );

        client_messages.insert(
            "client2".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"client2\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"channel1\"},\"cid\":2}".into()),
                TestMessage::Expect("{\"rid\":2}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Hello channel1\"}".into()),
                // Should not receive message from channel2
            ],
        );

        run_multi_client_test(client_messages).await;
    }

    #[tokio::test]
    async fn test_subscribe_unsubscribe_resubscribe() {
        let mut client_messages = HashMap::new();

        client_messages.insert(
            "client1".to_string(),
            vec![
                TestMessage::Send("{\"event\":\"#handshake\",\"data\":{\"authToken\":null},\"cid\":1}".into()),
                TestMessage::Expect("{\"data\":{\"id\":\"client1\",\"isAuthenticated\":false,\"pingTimeout\":10000},\"rid\":1}".into()),
                // Subscribe
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"test-channel\"},\"cid\":2}".into()),
                TestMessage::Expect("{\"rid\":2}".into()),
                // Publish and receive
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"test-channel\",\"data\":\"Message 1\"},\"cid\":3}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Message 1\"}".into()),
                TestMessage::Expect("{\"rid\":3}".into()),
                // Unsubscribe
                TestMessage::Send("{\"event\":\"#unsubscribe\",\"data\":\"test-channel\",\"cid\":4}".into()),
                TestMessage::Expect("{\"rid\":4}".into()),
                // Publish and don't receive
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"test-channel\",\"data\":\"Message 2\"},\"cid\":5}".into()),
                TestMessage::Expect("{\"rid\":5}".into()),
                // Resubscribe
                TestMessage::Send("{\"event\":\"#subscribe\",\"data\":{\"channel\":\"test-channel\"},\"cid\":6}".into()),
                TestMessage::Expect("{\"rid\":6}".into()),
                // Publish and receive again
                TestMessage::Send("{\"event\":\"#publish\",\"data\":{\"channel\":\"test-channel\",\"data\":\"Message 3\"},\"cid\":7}".into()),
                TestMessage::Expect("{\"event\":\"#publish\",\"data\":\"Message 3\"}".into()),
                TestMessage::Expect("{\"rid\":7}".into()),
            ],
        );

        run_multi_client_test(client_messages).await;
    }
}
