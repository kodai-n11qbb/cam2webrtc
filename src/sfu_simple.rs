use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::signaling::{SignalingMessage, SignalingMessageType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleSfuRoom {
    pub id: String,
    pub sender_id: Option<String>,
    pub viewer_ids: Vec<String>,
    pub sender_sdp: Option<String>,
    pub viewer_sdp: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SimpleSfuConnection {
    pub id: String,
    pub is_sender: bool,
    pub sdp_offer: Option<String>,
    pub sdp_answer: Option<String>,
}

pub struct SimpleSfuManager {
    rooms: Arc<RwLock<HashMap<String, SimpleSfuRoom>>>,
    connections: Arc<RwLock<HashMap<String, SimpleSfuConnection>>>,
}

impl SimpleSfuManager {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn handle_message(&self, room_id: &str, message: &SignalingMessage) -> Result<Vec<SignalingMessage>> {
        match message.message_type {
            SignalingMessageType::Join => {
                let connection_id = message.connection_id.as_ref().ok_or_else(|| anyhow::anyhow!("No connection_id"))?;
                let is_sender = message.is_sender.unwrap_or(false);
                self.add_connection(room_id, connection_id.clone(), is_sender).await
            }
            SignalingMessageType::Offer => {
                let connection_id = message.connection_id.as_ref().ok_or_else(|| anyhow::anyhow!("No connection_id"))?;
                let sdp = message.data.as_ref()
                    .and_then(|d| d.get("sdp"))
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow::anyhow!("No SDP in offer"))?;
                self.handle_sdp(room_id, connection_id, sdp, true).await
            }
            SignalingMessageType::Answer => {
                let connection_id = message.connection_id.as_ref().ok_or_else(|| anyhow::anyhow!("No connection_id"))?;
                let sdp = message.data.as_ref()
                    .and_then(|d| d.get("sdp"))
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow::anyhow!("No SDP in answer"))?;
                self.handle_sdp(room_id, connection_id, sdp, false).await
            }
            SignalingMessageType::IceCandidate => {
                let connection_id = message.connection_id.as_ref().ok_or_else(|| anyhow::anyhow!("No connection_id"))?;
                let candidate = message.data.as_ref()
                    .and_then(|d| d.get("candidate"))
                    .and_then(|c| c.as_str())
                    .ok_or_else(|| anyhow::anyhow!("No candidate"))?;
                self.handle_ice_candidate(room_id, connection_id, candidate).await
            }
            _ => Ok(Vec::new())
        }
    }

    pub async fn create_room(&self, room_id: String) -> Result<()> {
        let room = SimpleSfuRoom {
            id: room_id.clone(),
            sender_id: None,
            viewer_ids: Vec::new(),
            sender_sdp: None,
            viewer_sdp: HashMap::new(),
        };

        let mut rooms = self.rooms.write().await;
        rooms.insert(room_id, room);
        Ok(())
    }

    pub async fn add_connection(&self, room_id: &str, connection_id: String, is_sender: bool) -> Result<Vec<SignalingMessage>> {
        let connection = SimpleSfuConnection {
            id: connection_id.clone(),
            is_sender,
            sdp_offer: None,
            sdp_answer: None,
        };

        let mut connections = self.connections.write().await;
        connections.insert(connection_id.clone(), connection);

        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            if is_sender {
                room.sender_id = Some(connection_id.clone());
            } else {
                room.viewer_ids.push(connection_id.clone());
            }
        }

        let mut responses = Vec::new();

        // If this is a viewer and there's already a sender, send the sender's SDP
        if !is_sender {
            let rooms_guard = self.rooms.read().await;
            if let Some(room) = rooms_guard.get(room_id) {
                if let (Some(sender_id), Some(sender_sdp)) = (&room.sender_id, &room.sender_sdp) {
                    responses.push(SignalingMessage {
                        message_type: SignalingMessageType::Offer,
                        connection_id: Some(connection_id),
                        sender_id: Some(sender_id.clone()),
                        offer_id: Some(Uuid::new_v4().to_string()),
                        data: Some(serde_json::json!({ "sdp": sender_sdp })),
                        is_sender: Some(false),
                    });
                }
            }
        }

        Ok(responses)
    }

    pub async fn handle_sdp(&self, room_id: &str, connection_id: &str, sdp: &str, is_offer: bool) -> Result<Vec<SignalingMessage>> {
        let mut responses = Vec::new();

        if is_offer {
            // Store the SDP offer
            let mut connections = self.connections.write().await;
            if let Some(connection) = connections.get_mut(connection_id) {
                connection.sdp_offer = Some(sdp.to_string());
            }

            let mut rooms = self.rooms.write().await;
            if let Some(room) = rooms.get_mut(room_id) {
                room.sender_sdp = Some(sdp.to_string());
            }

            // Send offer to all viewers
            let rooms_guard = self.rooms.read().await;
            if let Some(room) = rooms_guard.get(room_id) {
                for viewer_id in &room.viewer_ids {
                    responses.push(SignalingMessage {
                        message_type: SignalingMessageType::Offer,
                        connection_id: Some(viewer_id.clone()),
                        sender_id: Some(connection_id.to_string()),
                        offer_id: Some(Uuid::new_v4().to_string()),
                        data: Some(serde_json::json!({ "sdp": sdp })),
                        is_sender: Some(false),
                    });
                }
            }
        } else {
            // Store the SDP answer
            let mut connections = self.connections.write().await;
            if let Some(connection) = connections.get_mut(connection_id) {
                connection.sdp_answer = Some(sdp.to_string());
            }

            // In a real SFU, we would process the answer and establish the WebRTC connection
            // For now, we just acknowledge it
        }

        Ok(responses)
    }

    pub async fn handle_ice_candidate(&self, room_id: &str, connection_id: &str, candidate: &str) -> Result<Vec<SignalingMessage>> {
        let mut responses = Vec::new();

        // In a real SFU, we would handle ICE candidates between sender and viewers
        // For now, we just forward them
        let rooms_guard = self.rooms.read().await;
        if let Some(room) = rooms_guard.get(room_id) {
            let is_sender = room.sender_id.as_ref().map(|s| s.as_str()) == Some(connection_id);

            if is_sender {
                // Forward sender's ICE candidates to all viewers
                for viewer_id in &room.viewer_ids {
                    responses.push(SignalingMessage {
                        message_type: SignalingMessageType::IceCandidate,
                        connection_id: Some(viewer_id.clone()),
                        sender_id: Some(connection_id.to_string()),
                        offer_id: None,
                        data: Some(serde_json::json!({ "candidate": candidate })),
                        is_sender: None,
                    });
                }
            } else {
                // Forward viewer's ICE candidates to sender
                if let Some(sender_id) = &room.sender_id {
                    responses.push(SignalingMessage {
                        message_type: SignalingMessageType::IceCandidate,
                        connection_id: Some(sender_id.clone()),
                        sender_id: Some(connection_id.to_string()),
                        offer_id: None,
                        data: Some(serde_json::json!({ "candidate": candidate })),
                        is_sender: None,
                    });
                }
            }
        }

        Ok(responses)
    }

    pub async fn remove_connection(&self, room_id: &str, connection_id: &str) -> Result<Vec<SignalingMessage>> {
        let mut connections = self.connections.write().await;
        connections.remove(connection_id);

        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            if room.sender_id.as_ref().map(|s| s.as_str()) == Some(connection_id) {
                room.sender_id = None;
                room.sender_sdp = None;
            }
            room.viewer_ids.retain(|id| id != connection_id);
            room.viewer_sdp.remove(connection_id);
        }

        // Notify other participants
        let rooms_guard = self.rooms.read().await;
        let mut responses = Vec::new();
        if let Some(room) = rooms_guard.get(room_id) {
            let participant_ids: Vec<String> = room.viewer_ids.iter()
                .chain(room.sender_id.iter())
                .filter(|id| *id != connection_id)
                .cloned()
                .collect();

            for participant_id in participant_ids {
                responses.push(SignalingMessage {
                    message_type: SignalingMessageType::Leave,
                    connection_id: Some(participant_id),
                    sender_id: None,
                    offer_id: None,
                    data: Some(serde_json::json!({
                        "connection_id": connection_id,
                        "connection_count": room.viewer_ids.len() + if room.sender_id.is_some() { 1 } else { 0 }
                    })),
                    is_sender: None,
                });
            }
        }

        Ok(responses)
    }
}
