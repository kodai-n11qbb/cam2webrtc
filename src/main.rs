use log::{info, error};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use warp::{Filter, Reply};
use warp::ws::{WebSocket, Message};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod room;
mod stun;
mod turn;
mod signaling;

use room::{Room, RoomManager};
use signaling::SignalingMessage;
use stun::StunServer;
use turn::TurnServer;

// Type alias for Clients map: connection_id -> sender channel
type Clients = Arc<RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomResponse {
    room_id: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    
    info!("Starting Cam2WebRTC Signaling Server...");

    // Start STUN server
    tokio::task::spawn(async move {
        let stun_addr = "0.0.0.0:3478".parse().unwrap();
        match StunServer::new(stun_addr) {
            Ok(mut server) => {
                info!("Starting STUN server on {}", stun_addr);
                if let Err(e) = server.run().await {
                    error!("STUN server failed: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to create STUN server: {}", e);
            }
        }
    });

    // Start TURN server
    tokio::task::spawn(async move {
        let turn_addr = "0.0.0.0:3479".parse().unwrap();
        match TurnServer::new(turn_addr) {
            Ok(mut server) => {
                info!("Starting TURN server on {}", turn_addr);
                if let Err(e) = server.run().await {
                    error!("TURN server failed: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to create TURN server: {}", e);
            }
        }
    });
    
    // Initialize room manager
    let room_manager = Arc::new(RwLock::new(RoomManager::new()));
    
    // Initialize clients map
    let clients = Clients::default();
    
    // Clone for WebSocket handler
    let room_manager_ws = room_manager.clone();
    let clients_ws = clients.clone();
    
    // WebSocket route
    let ws_route = warp::path("ws")
        .and(warp::path::param::<String>())
        .and(warp::ws())
        .and(warp::any().map(move || room_manager_ws.clone()))
        .and(warp::any().map(move || clients_ws.clone()))
        .and_then(|room_id: String, ws: warp::ws::Ws, room_manager: Arc<RwLock<RoomManager>>, clients: Clients| async move {
            Ok::<_, warp::Rejection>(ws.on_upgrade(move |socket| handle_websocket(socket, room_id, room_manager, clients)))
        });
    
    // REST API routes
    let room_manager_api = room_manager.clone();
    let room_manager_get = room_manager.clone();
    
    let rooms_base = warp::path("api").and(warp::path("rooms"));

    let create_room_route = rooms_base
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || room_manager_api.clone()))
        .and_then(|_req: CreateRoomRequest, room_manager: Arc<RwLock<RoomManager>>| async move {
            let room_id = Uuid::new_v4().to_string();
            let mut manager = room_manager.write().await;
            
            manager.create_room(room_id.clone());
            
            let response = RoomResponse {
                room_id,
            };
            
            Ok::<_, warp::Rejection>(warp::reply::json(&response))
        });

    let get_room_route = rooms_base
        .and(warp::path::param::<String>())
        .and(warp::get())
        .and(warp::any().map(move || room_manager_get.clone()))
        .and_then(|room_id: String, room_manager: Arc<RwLock<RoomManager>>| async move {
            let manager = room_manager.read().await;
            if manager.rooms.contains_key(&room_id) {
                 Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({"exists": true})))
            } else {
                Err(warp::reject::not_found())
            }
        });
    
    let api_routes = create_room_route.or(get_room_route);
    
    // Static file serving for HTML clients
    let static_files = warp::fs::dir("static");
    
    // Combine all routes
    let routes = ws_route
        .or(api_routes)
        .or(static_files)
        .with(warp::cors().allow_any_origin().allow_methods(vec!["GET", "POST"]));
    
    info!("Server listening on 0.0.0.0:8080");
    
    // Start server
    warp::serve(routes)
        .run(([0, 0, 0, 0], 8080))
        .await;
    
    Ok(())
}

async fn handle_websocket(
    socket: WebSocket,
    room_id: String,
    room_manager: Arc<RwLock<RoomManager>>,
    clients: Clients,
) {
    info!("New WebSocket connection for room: {}", room_id);
    
    let (mut user_ws_tx, mut user_ws_rx) = socket.split();
    
    // Create channel for this client
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    
    // Spawn task to forward messages from channel to WebSocket
    tokio::task::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let Err(e) = user_ws_tx.send(message).await {
                error!("Websocket send error: {}", e);
                break;
            }
        }
    });

    let room_manager_clone = room_manager.clone();
    let clients_clone = clients.clone();
    let mut current_connection_id: Option<String> = None;
    
    // Handle incoming messages
    while let Some(result) = user_ws_rx.next().await {
        match result {
            Ok(msg) => {
                if let Ok(text) = msg.to_str() {
                    if let Ok(signaling_msg) = serde_json::from_str::<SignalingMessage>(text) {
                        // Track connection_id from messages
                        // If we don't have a connection_id yet, try to get it from the message
                        if current_connection_id.is_none() {
                            if let Some(ref cid) = signaling_msg.connection_id {
                                current_connection_id = Some(cid.clone());
                                // Register client
                                clients_clone.write().await.insert(cid.clone(), tx.clone());
                                info!("Registered client: {}", cid);
                            }
                        }

                        let mut manager = room_manager_clone.write().await;
                        if let Some(responses) = manager.handle_message(room_id.clone(), signaling_msg) {
                            for response in responses {
                                if let Ok(response_text) = serde_json::to_string(&response) {
                                    // Route response to target connection_id
                                    if let Some(target_id) = &response.connection_id {
                                        let clients_guard = clients_clone.read().await;
                                        if let Some(target_tx) = clients_guard.get(target_id) {
                                            let _ = target_tx.send(Message::text(response_text));
                                        } else {
                                            // Fallback: if not found, maybe send to self if it matches? 
                                            // But room logic specifically sets target.
                                            // If target is missing, it might have disconnected.
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
        }
    }
    
    // Clean up connection
    if let Some(cid) = current_connection_id {
        let mut manager = room_manager_clone.write().await;
        if let Some(responses) = manager.remove_connection(&room_id, &cid) {
            for response in responses {
                if let Ok(response_text) = serde_json::to_string(&response) {
                    if let Some(target_id) = &response.connection_id {
                        let clients_guard = clients_clone.read().await;
                        if let Some(target_tx) = clients_guard.get(target_id) {
                            let _ = target_tx.send(Message::text(response_text));
                        }
                    }
                }
            }
        }
        
        let mut clients_guard = clients_clone.write().await;
        clients_guard.remove(&cid);
        
        info!("WebSocket connection closed for room: {}, connection: {}", room_id, cid);
    } else {
        info!("WebSocket connection closed for room: {} (no connection_id established)", room_id);
    }
}
