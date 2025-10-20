use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;
use sync_core::{protocol::{ClientMessage, ServerMessage}, SyncResult, errors::ClientError};
use crate::events::EventDispatcher;
use backoff::{future::retry, ExponentialBackoff};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct WebSocketClient {
    tx: mpsc::Sender<ClientMessage>,
    is_connected: Arc<AtomicBool>
}

pub struct WebSocketReceiver {
    rx: mpsc::Receiver<ServerMessage>,
}

impl WebSocketClient {
    pub async fn connect(
        server_url: &str,
        email: &str,
        client_id: Uuid,
        api_key: &str,
        api_secret: &str,
        event_dispatcher: Option<Arc<EventDispatcher>>,
        is_connected: Arc<AtomicBool>
    ) -> SyncResult<(Self, WebSocketReceiver)> {
        // Delegate to connect_with_hmac (HMAC is now required)
        Self::connect_with_hmac(server_url, email, client_id, api_key, api_secret, event_dispatcher, is_connected).await
    }

    pub async fn connect_with_hmac(
        server_url: &str,
        email: &str,
        client_id: Uuid,
        api_key: &str,
        api_secret: &str,
        event_dispatcher: Option<Arc<EventDispatcher>>,
        is_connected: Arc<AtomicBool>   
    ) -> SyncResult<(Self, WebSocketReceiver)> {
        let ws_stream = Self::connect_with_retry(server_url, 3, event_dispatcher).await?;

        let (write, read) = ws_stream.split();

        // Create channels for communication
        let (tx_send, mut rx_send) = mpsc::channel::<ClientMessage>(100);
        let (tx_recv, rx_recv) = mpsc::channel::<ServerMessage>(100);

        // Spawn writer task
        is_connected.store(true, std::sync::atomic::Ordering::Relaxed);
        let is_connected_d = is_connected.clone();
        tokio::spawn(async move {
            let mut write = write;
            while let Some(msg) = rx_send.recv().await {
                let json = serde_json::to_string(&msg).unwrap();
                if write.send(Message::Text(json)).await.is_err() {
                    is_connected_d.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });

        // Spawn reader task
        let is_connected_d = is_connected.clone();
        tokio::spawn(async move {
            let mut read = read;
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                            if tx_recv.send(server_msg).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        is_connected_d.store(false, std::sync::atomic::Ordering::Relaxed);
                    },
                    _ => {}
                }
            }
        });

        let client = Self {
            tx: tx_send.clone(),
            is_connected: is_connected.clone()
        };

        let receiver = WebSocketReceiver {
            rx: rx_recv,
        };

        // Create timestamp
        let timestamp = chrono::Utc::now().timestamp();

        // Create HMAC signature
        let signature = Self::create_hmac_signature(
            api_secret,
            timestamp,
            email,
            api_key,
            "", // Empty body for auth
        );

        // Send authentication with HMAC signature
        client.send(ClientMessage::Authenticate {
            email: email.to_string(),
            client_id,
            api_key: Some(api_key.to_string()),
            signature: Some(signature),
            timestamp: Some(timestamp),
        }).await?;

        Ok((client, receiver))
    }
    
    async fn connect_with_retry(
        server_url: &str, 
        _max_retries: u32,
        event_dispatcher: Option<Arc<EventDispatcher>>,
    ) -> SyncResult<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
        let backoff = ExponentialBackoff {
            initial_interval: std::time::Duration::from_millis(100),
            max_interval: std::time::Duration::from_millis(2000),
            max_elapsed_time: Some(std::time::Duration::from_secs(10)),
            randomization_factor: 0.1, // Add 10% jitter
            ..Default::default()
        };
        
        let server_url = server_url.to_string();
        let dispatcher = event_dispatcher.clone();
        let operation = || async {
            // Emit connection attempt event
            if let Some(ref dispatcher) = dispatcher {
                dispatcher.emit_connection_attempted(&server_url);
            }
            
            match connect_async(&server_url).await {
                Ok((ws_stream, _)) => {
                    // Emit connection success event
                    if let Some(ref dispatcher) = dispatcher {
                        dispatcher.emit_connection_succeeded(&server_url);
                    }
                    Ok(ws_stream)
                },
                Err(e) => {
                    // Emit as sync error instead of tracing warning
                    if let Some(ref dispatcher) = dispatcher {
                        dispatcher.emit_sync_error(&format!("Connection failed: {}", e));
                    }
                    Err(backoff::Error::transient(e))
                }
            }
        };
        
        retry(backoff, operation)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()).into())
    }
    
    pub async fn send(&self, message: ClientMessage) -> SyncResult<()> {
        self.tx
            .send(message)
            .await
            .map_err(|_| ClientError::WebSocket("Failed to send message".to_string()).into())
    }

    fn create_hmac_signature(
        secret: &str,
        timestamp: i64,
        email: &str,
        api_key: &str,
        body: &str,
    ) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC can take key of any size");

        let message = format!("{}.{}.{}.{}", timestamp, email, api_key, body);
        mac.update(message.as_bytes());

        hex::encode(mac.finalize().into_bytes())
    }
}

impl WebSocketReceiver {
    pub async fn receive(&mut self) -> SyncResult<Option<ServerMessage>> {
        Ok(self.rx.recv().await)
    }
    
    pub async fn forward_to(mut self, tx: mpsc::Sender<ServerMessage>) -> SyncResult<()> {
        tracing::info!("CLIENT: WebSocket receiver forwarder started");
        while let Some(msg) = self.receive().await? {
            tracing::info!("CLIENT: Received WebSocket message: {:?}", std::mem::discriminant(&msg));
            if tx.send(msg).await.is_err() {
                tracing::error!("CLIENT: Failed to forward message to handler");
                break;
            } else {
                tracing::info!("CLIENT: Successfully forwarded message to handler");
            }
        }
        tracing::warn!("CLIENT: WebSocket receiver forwarder terminated");
        Ok(())
    }
    
}