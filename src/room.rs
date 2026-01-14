use std::collections::HashMap;
use uuid::Uuid;
use crate::signaling::{SignalingMessage, SignalingMessageType};

#[derive(Debug, Clone)]
pub struct Room {
    #[allow(dead_code)]
    pub id: String,
    pub connections: HashMap<String, ConnectionInfo>,
    pub offers: HashMap<String, SignalingMessage>,
}

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    #[allow(dead_code)]
    pub id: String,
    pub is_sender: bool,
    #[allow(dead_code)]
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

impl Room {
    pub fn new(id: String) -> Self {
        Self {
            id,
            connections: HashMap::new(),
            offers: HashMap::new(),
        }
    }
    
    pub fn add_connection(&mut self, connection_id: String, is_sender: bool) -> Result<Vec<String>, String> {
        let removed_ids = Vec::new();
        
        // If the new connection is a sender, we should check if one already exists
        // (Usually only 1 sender per room in this simple model)
        if is_sender {
            let sender_exists = self.connections.values().any(|c| c.is_sender);
            if sender_exists {
                // For simplicity, we could allow it, but let's stick to 1 sender
                return Err("Sender already exists in this room".to_string());
            }
        }
        
        let connection_info = ConnectionInfo {
            id: connection_id.clone(),
            is_sender,
            connected_at: chrono::Utc::now(),
        };
        
        self.connections.insert(connection_id, connection_info);
        Ok(removed_ids)
    }
    
    pub fn remove_connection(&mut self, connection_id: &str) {
        self.connections.remove(connection_id);
        // Clean up associated offers
        self.offers.retain(|_, offer| {
            if let Some(sender_id) = offer.sender_id.as_ref() {
                sender_id != connection_id
            } else {
                true
            }
        });
    }
    
    pub fn add_offer(&mut self, offer: SignalingMessage) -> Result<(), String> {
        let offer_id = Uuid::new_v4().to_string();
        let mut offer_with_id = offer;
        offer_with_id.offer_id = Some(offer_id.clone());
        
        self.offers.insert(offer_id, offer_with_id);
        Ok(())
    }
    
    pub fn get_offers_for_viewer(&self) -> Vec<&SignalingMessage> {
        self.offers.values().collect()
    }
    
    pub fn get_connection_count(&self) -> usize {
        self.connections.len()
    }
}

#[derive(Debug)]
pub struct RoomManager {
    pub rooms: HashMap<String, Room>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            rooms: HashMap::new(),
        }
    }
    
    pub fn create_room(&mut self, room_id: String) {
        let room = Room::new(room_id.clone());
        self.rooms.insert(room_id, room);
    }
    
    pub fn handle_message(&mut self, room_id: String, message: SignalingMessage) -> Option<Vec<SignalingMessage>> {
        let room = self.rooms.get_mut(&room_id)?;
        
        match message.message_type {
            SignalingMessageType::Join => {
                let is_sender = message.is_sender.unwrap_or(false);
                let connection_id = message.connection_id.clone()?;
                
                let removed_ids = match room.add_connection(connection_id.clone(), is_sender) {
                    Ok(ids) => ids,
                    Err(e) => {
                        return Some(vec![SignalingMessage {
                            message_type: SignalingMessageType::Error,
                            connection_id: Some(connection_id),
                            sender_id: None,
                            offer_id: None,
                            data: Some(serde_json::json!({
                                "error": e
                            })),
                            is_sender: None,
                        }]);
                    }
                };
                
                let connection_count = room.get_connection_count();

                // Prepare RoomInfo for the joiner
                let mut responses = vec![SignalingMessage {
                    message_type: SignalingMessageType::RoomInfo,
                    connection_id: Some(connection_id.clone()),
                    sender_id: None,
                    offer_id: None,
                    data: Some(serde_json::json!({
                        "room_id": room_id,
                        "mode": "1onN",
                        "connection_count": connection_count,
                        "peers": room.connections.iter()
                                .filter(|(id, _)| *id != &connection_id)
                                .map(|(id, info)| serde_json::json!({ "id": id, "is_sender": info.is_sender }))
                                .collect::<Vec<_>>()
                    })),
                    is_sender: None,
                }];

                // Notify about replaced connections (Leave messages)
                for rid in removed_ids {
                    for (other_id, _) in &room.connections {
                        responses.push(SignalingMessage {
                            message_type: SignalingMessageType::Leave,
                            connection_id: Some(other_id.clone()),
                            sender_id: None,
                            offer_id: None,
                            data: Some(serde_json::json!({
                                "connection_id": rid,
                                "connection_count": connection_count
                            })),
                            is_sender: None,
                        });
                    }
                }

                // Notify other peers about the new user
                for (other_id, _) in &room.connections {
                    if *other_id != connection_id {
                        responses.push(SignalingMessage {
                            message_type: SignalingMessageType::NewPeer,
                            connection_id: Some(other_id.clone()),
                            sender_id: None,
                            offer_id: None,
                            data: Some(serde_json::json!({
                                "connection_id": connection_id,
                                "is_sender": is_sender,
                                "connection_count": connection_count
                            })),
                            is_sender: None,
                        });
                    }
                }

                // Legacy: If this is a viewer, send them existing stored offers
                if !is_sender {
                    let offers = room.get_offers_for_viewer();
                    for offer in offers {
                        responses.push(SignalingMessage {
                            message_type: SignalingMessageType::Offer,
                            connection_id: Some(connection_id.clone()),
                            sender_id: offer.sender_id.clone(),
                            offer_id: offer.offer_id.clone(),
                            data: offer.data.clone(),
                            is_sender: None,
                        });
                    }
                }
                
                Some(responses)
            }
            
            SignalingMessageType::Offer => {
                // In Mesh 1onN, we usually route directly if connection_id is set
                if message.connection_id.is_some() {
                    return Some(vec![message]);
                }

                // Store and broadcast (Legacy/Broadcast Mode support)
                if let Err(e) = room.add_offer(message.clone()) {
                    return Some(vec![SignalingMessage {
                        message_type: SignalingMessageType::Error,
                        connection_id: message.connection_id,
                        sender_id: message.sender_id,
                        offer_id: message.offer_id,
                        data: Some(serde_json::json!({
                            "error": e
                        })),
                        is_sender: None,
                    }]);
                }
                
                let offers = room.get_offers_for_viewer();
                let mut responses = Vec::new();
                
                for offer in offers {
                    for (conn_id, conn_info) in &room.connections {
                        if !conn_info.is_sender {
                            responses.push(SignalingMessage {
                                message_type: SignalingMessageType::Offer,
                                connection_id: Some(conn_id.clone()),
                                sender_id: offer.sender_id.clone(),
                                offer_id: offer.offer_id.clone(),
                                data: offer.data.clone(),
                                is_sender: None,
                            });
                        }
                    }
                }
                
                Some(responses)
            }
            
            SignalingMessageType::Answer => Some(vec![message]),

            SignalingMessageType::IceCandidate => {
                if message.connection_id.is_some() {
                    Some(vec![message])
                } else {
                    let mut responses = Vec::new();
                    for (conn_id, conn_info) in &room.connections {
                        if !conn_info.is_sender {
                            let mut msg = message.clone();
                            msg.connection_id = Some(conn_id.clone());
                            responses.push(msg);
                        }
                    }
                    Some(responses)
                }
            }
            
            _ => None,
        }
    }
    
    pub fn remove_connection(&mut self, room_id: &str, connection_id: &str) -> Option<Vec<SignalingMessage>> {
        let room = self.rooms.get_mut(room_id)?;
        room.remove_connection(connection_id);
        
        let connection_count = room.get_connection_count();
        let mut responses = Vec::new();
        
        for (other_id, _) in &room.connections {
            responses.push(SignalingMessage {
                message_type: SignalingMessageType::Leave,
                connection_id: Some(other_id.clone()),
                sender_id: None,
                offer_id: None,
                data: Some(serde_json::json!({
                    "connection_id": connection_id,
                    "connection_count": connection_count
                })),
                is_sender: None,
            });
        }
        
        Some(responses)
    }
}

