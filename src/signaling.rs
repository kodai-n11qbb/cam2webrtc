use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalingMessage {
    #[serde(rename = "type")]
    pub message_type: SignalingMessageType,
    pub connection_id: Option<String>,
    pub sender_id: Option<String>,
    pub offer_id: Option<String>,
    pub data: Option<Value>,
    pub is_sender: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalingMessageType {
    Join,
    Leave,
    Offer,
    Answer,
    IceCandidate,
    RoomInfo,
    Error,
    NewPeer,
}

impl SignalingMessage {
    pub fn new_join(connection_id: String, is_sender: bool) -> Self {
        Self {
            message_type: SignalingMessageType::Join,
            connection_id: Some(connection_id),
            sender_id: None,
            offer_id: None,
            data: None,
            is_sender: Some(is_sender),
        }
    }
    
    pub fn new_offer(
        connection_id: String,
        sender_id: String,
        sdp: Value,
    ) -> Self {
        Self {
            message_type: SignalingMessageType::Offer,
            connection_id: Some(connection_id),
            sender_id: Some(sender_id),
            offer_id: None,
            data: Some(sdp),
            is_sender: Some(true),
        }
    }
    
    pub fn new_answer(
        connection_id: String,
        sender_id: String,
        sdp: Value,
    ) -> Self {
        Self {
            message_type: SignalingMessageType::Answer,
            connection_id: Some(connection_id),
            sender_id: Some(sender_id),
            offer_id: None,
            data: Some(sdp),
            is_sender: Some(false),
        }
    }
    
    pub fn new_ice_candidate(
        connection_id: String,
        sender_id: String,
        candidate: Value,
    ) -> Self {
        Self {
            message_type: SignalingMessageType::IceCandidate,
            connection_id: Some(connection_id),
            sender_id: Some(sender_id),
            offer_id: None,
            data: Some(candidate),
            is_sender: None,
        }
    }
    
    pub fn new_error(connection_id: String, error: String) -> Self {
        Self {
            message_type: SignalingMessageType::Error,
            connection_id: Some(connection_id),
            sender_id: None,
            offer_id: None,
            data: Some(serde_json::json!({
                "error": error
            })),
            is_sender: None,
        }
    }
}

pub struct SignalingServer {
    // Additional signaling server logic can be added here
}

impl SignalingServer {
    pub fn new() -> Self {
        Self {}
    }
}
