use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::errors::ClientError;
use backoff::{future::retry, ExponentialBackoff};

#[derive(Clone)]
pub struct WebSocketClient {
    tx: mpsc::Sender<ClientMessage>,
}

pub struct WebSocketReceiver {
    rx: mpsc::Receiver<ServerMessage>,
}

impl WebSocketClient {
    pub async fn connect(
        server_url: &str,
        user_id: Uuid,
        auth_token: &str,
    ) -> Result<(Self, WebSocketReceiver), ClientError> {
        let ws_stream = Self::connect_with_retry(server_url, 3).await?;
        
        let (write, read) = ws_stream.split();
        
        // Create channels for communication
        let (tx_send, mut rx_send) = mpsc::channel::<ClientMessage>(100);
        let (tx_recv, rx_recv) = mpsc::channel::<ServerMessage>(100);
        
        // Spawn writer task
        tokio::spawn(async move {
            let mut write = write;
            while let Some(msg) = rx_send.recv().await {
                let json = serde_json::to_string(&msg).unwrap();
                if write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        });
        
        // Spawn reader task
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
                    Ok(Message::Close(_)) => break,
                    _ => {}
                }
            }
        });
        
        let client = Self {
            tx: tx_send.clone(),
        };
        
        let receiver = WebSocketReceiver {
            rx: rx_recv,
        };
        
        // Send authentication
        client.send(ClientMessage::Authenticate {
            user_id,
            auth_token: auth_token.to_string(),
        }).await?;
        
        Ok((client, receiver))
    }
    
    async fn connect_with_retry(
        server_url: &str, 
        max_retries: u32
    ) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, ClientError> {
        let backoff = ExponentialBackoff {
            initial_interval: std::time::Duration::from_millis(100),
            max_interval: std::time::Duration::from_millis(2000),
            max_elapsed_time: Some(std::time::Duration::from_secs(10)),
            randomization_factor: 0.1, // Add 10% jitter
            ..Default::default()
        };
        
        let server_url = server_url.to_string();
        let operation = || async {
            match connect_async(&server_url).await {
                Ok((ws_stream, _)) => Ok(ws_stream),
                Err(e) => {
                    tracing::warn!("WebSocket connection failed, will retry: {}", e);
                    Err(backoff::Error::transient(e))
                }
            }
        };
        
        retry(backoff, operation)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))
    }
    
    pub async fn send(&self, message: ClientMessage) -> Result<(), ClientError> {
        self.tx
            .send(message)
            .await
            .map_err(|_| ClientError::WebSocket("Failed to send message".to_string()))?;
        Ok(())
    }
}

impl WebSocketReceiver {
    pub async fn receive(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        Ok(self.rx.recv().await)
    }
    
    pub async fn forward_to(mut self, tx: mpsc::Sender<ServerMessage>) -> Result<(), ClientError> {
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