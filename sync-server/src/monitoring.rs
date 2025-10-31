use chrono::Local;
use colored::*;
use sync_core::protocol::{ClientMessage, ServerMessage};
use tokio::sync::mpsc;
use tracing::info;

#[derive(Debug, Clone)]
pub enum LogMessage {
    ClientConnected {
        client_id: String,
    },
    ClientDisconnected {
        client_id: String,
    },
    MessageReceived {
        client_id: String,
        message: ClientMessage,
    },
    MessageSent {
        client_id: String,
        message: ServerMessage,
    },
    PatchApplied {
        document_id: String,
        patch: String,
    },
    ConflictDetected {
        document_id: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone)]
pub struct MonitoringLayer {
    tx: mpsc::Sender<LogMessage>,
}

impl MonitoringLayer {
    pub fn new(tx: mpsc::Sender<LogMessage>) -> Self {
        Self { tx }
    }

    pub async fn log_client_connected(&self, client_id: &str) {
        let _ = self
            .tx
            .send(LogMessage::ClientConnected {
                client_id: client_id.to_string(),
            })
            .await;
    }

    pub async fn log_client_disconnected(&self, client_id: &str) {
        let _ = self
            .tx
            .send(LogMessage::ClientDisconnected {
                client_id: client_id.to_string(),
            })
            .await;
    }

    pub async fn log_message_received(&self, client_id: &str, message: ClientMessage) {
        let _ = self
            .tx
            .send(LogMessage::MessageReceived {
                client_id: client_id.to_string(),
                message,
            })
            .await;
    }

    pub async fn log_message_sent(&self, client_id: &str, message: ServerMessage) {
        let _ = self
            .tx
            .send(LogMessage::MessageSent {
                client_id: client_id.to_string(),
                message,
            })
            .await;
    }

    pub async fn log_patch_applied(&self, document_id: &str, patch: &serde_json::Value) {
        let patch_json = serde_json::to_string_pretty(patch).unwrap_or_default();
        let _ = self
            .tx
            .send(LogMessage::PatchApplied {
                document_id: document_id.to_string(),
                patch: patch_json,
            })
            .await;
    }

    pub async fn log_conflict_detected(&self, document_id: &str) {
        let _ = self
            .tx
            .send(LogMessage::ConflictDetected {
                document_id: document_id.to_string(),
            })
            .await;
    }

    pub async fn log_error(&self, message: String) {
        let _ = self.tx.send(LogMessage::Error { message }).await;
    }
}

pub async fn spawn_monitoring_display(mut rx: mpsc::Receiver<LogMessage>) {
    tokio::spawn(async move {
        info!("");
        info!("{}", "📋 Activity Log:".bold());
        info!("{}", "─".repeat(80).dimmed());

        while let Some(log) = rx.recv().await {
            let timestamp = Local::now().format("%H:%M:%S%.3f");

            match log {
                LogMessage::ClientConnected { client_id } => {
                    info!(
                        "{} {} Client connected: {}",
                        timestamp.to_string().dimmed(),
                        "→".green().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::ClientDisconnected { client_id } => {
                    info!(
                        "{} {} Client disconnected: {}",
                        timestamp.to_string().dimmed(),
                        "←".red().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::MessageReceived { client_id, message } => {
                    let msg_type = match message {
                        ClientMessage::Authenticate { .. } => "Authenticate",
                        ClientMessage::CreateDocument { .. } => "CreateDocument",
                        ClientMessage::UpdateDocument { .. } => "UpdateDocument",
                        ClientMessage::DeleteDocument { .. } => "DeleteDocument",
                        ClientMessage::RequestSync { .. } => "RequestSync",
                        ClientMessage::RequestFullSync => "RequestFullSync",
                        ClientMessage::Ping => "Ping",
                        ClientMessage::GetChangesSince { .. } => "GetChangesSince",
                        ClientMessage::AckChanges { .. } => "AckChanges",
                    };
                    info!(
                        "{} {} {} from {}",
                        timestamp.to_string().dimmed(),
                        "↓".blue(),
                        msg_type.white().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::MessageSent { client_id, message } => {
                    let msg_type = match message {
                        ServerMessage::AuthSuccess { .. } => "AuthSuccess",
                        ServerMessage::AuthError { .. } => "AuthError",
                        ServerMessage::DocumentCreated { .. } => "DocumentCreated",
                        ServerMessage::DocumentUpdated { .. } => "DocumentUpdated",
                        ServerMessage::DocumentDeleted { .. } => "DocumentDeleted",
                        ServerMessage::DocumentCreatedResponse { .. } => "DocumentCreatedResponse",
                        ServerMessage::DocumentUpdatedResponse { .. } => "DocumentUpdatedResponse",
                        ServerMessage::DocumentDeletedResponse { .. } => "DocumentDeletedResponse",
                        ServerMessage::SyncDocument { .. } => "SyncDocument",
                        ServerMessage::SyncComplete { .. } => "SyncComplete",
                        ServerMessage::ConflictDetected { .. } => "ConflictDetected",
                        ServerMessage::Error { .. } => "Error",
                        ServerMessage::Pong => "Pong",
                        ServerMessage::Changes { .. } => "Changes",
                        ServerMessage::ChangesAcknowledged { .. } => "ChangesAcknowledged",
                    };
                    info!(
                        "{} {} {} to {}",
                        timestamp.to_string().dimmed(),
                        "↑".green(),
                        msg_type.white().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::PatchApplied { document_id, patch } => {
                    println!(
                        "{} 🔧 Patch applied to document {}:",
                        timestamp.to_string().dimmed(),
                        document_id.blue()
                    );
                    // Print patch with indentation
                    for line in patch.lines() {
                        info!("     {}", line.cyan());
                    }
                }
                LogMessage::ConflictDetected { document_id } => {
                    println!(
                        "{} ⚠️ Conflict detected for document {}",
                        timestamp.to_string().dimmed(),
                        document_id.red().bold()
                    );
                }
                LogMessage::Error { message } => {
                    println!(
                        "{} ❌ Error: {}",
                        timestamp.to_string().dimmed(),
                        message.red()
                    );
                }
            }
        }
    });
}
