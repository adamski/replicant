use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::errors::ClientError;

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
        let (ws_stream, _) = connect_async(server_url)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        
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
        while let Some(msg) = self.receive().await? {
            if tx.send(msg).await.is_err() {
                break;
            }
        }
        Ok(())
    }
    
}