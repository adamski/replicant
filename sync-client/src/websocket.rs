use tokio_tungstenite::{connect_async, tungstenite::Message, WebSocketStream, MaybeTlsStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::net::TcpStream;
use uuid::Uuid;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::errors::ClientError;

pub struct WebSocketClient {
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    server_url: String,
}

impl WebSocketClient {
    pub async fn connect(
        server_url: &str,
        user_id: Uuid,
        auth_token: &str,
    ) -> Result<Self, ClientError> {
        let (ws_stream, _) = connect_async(server_url)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        
        let mut client = Self {
            ws_stream,
            server_url: server_url.to_string(),
        };
        
        // Send authentication
        client.send(ClientMessage::Authenticate {
            user_id,
            auth_token: auth_token.to_string(),
        }).await?;
        
        Ok(client)
    }
    
    pub async fn send(&mut self, message: ClientMessage) -> Result<(), ClientError> {
        let json = serde_json::to_string(&message)?;
        self.ws_stream
            .send(Message::Text(json))
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        Ok(())
    }
    
    pub async fn receive(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        if let Some(msg) = self.ws_stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let server_msg = serde_json::from_str(&text)?;
                    Ok(Some(server_msg))
                }
                Ok(Message::Close(_)) => Ok(None),
                Ok(_) => Ok(None), // Ignore other message types
                Err(e) => Err(ClientError::WebSocket(e.to_string())),
            }
        } else {
            Ok(None)
        }
    }
    
    pub async fn start_reading(
        &mut self,
        tx: mpsc::Sender<ServerMessage>,
    ) -> Result<(), ClientError> {
        while let Some(msg) = self.receive().await? {
            if tx.send(msg).await.is_err() {
                break;
            }
        }
        Ok(())
    }
    
    pub async fn close(mut self) -> Result<(), ClientError> {
        self.ws_stream
            .close(None)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        Ok(())
    }
    
    pub async fn reconnect(&mut self, user_id: Uuid, auth_token: &str) -> Result<(), ClientError> {
        // Close existing connection
        let _ = self.ws_stream.close(None).await;
        
        // Reconnect
        let (ws_stream, _) = connect_async(&self.server_url)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        
        self.ws_stream = ws_stream;
        
        // Re-authenticate
        self.send(ClientMessage::Authenticate {
            user_id,
            auth_token: auth_token.to_string(),
        }).await?;
        
        Ok(())
    }
}