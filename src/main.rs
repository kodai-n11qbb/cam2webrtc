use log::{info, error};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use warp::Filter;
use warp::ws::{WebSocket, Message};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod room;
mod stun;
mod turn;
mod signaling;
mod config;
mod network;

use room::RoomManager;
use signaling::SignalingMessage;
use stun::StunServer;
use turn::TurnServer;
use config::Config;
use std::net::SocketAddr;
use std::fs;
use rcgen::generate_simple_self_signed;
use network::get_all_local_ips;

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
    
    let boot_time = std::time::Instant::now();
    
    info!("Starting Cam2WebRTC Signaling Server...");

    let config = Config::load("config.json").unwrap_or_else(|e| {
        error!("Failed to load config.json: {}. Using defaults.", e);
        Config {
            signaling_addr: "0.0.0.0:8080".to_string(),
            stun_addr: "0.0.0.0:3478".to_string(),
            turn_addr: "0.0.0.0:3479".to_string(),
            ice_servers: vec![config::IceServerConfig { urls: vec!["stun:localhost:3478".to_string()] }],
            video_constraints: serde_json::json!({
                "width": { "ideal": 1280 },
                "height": { "ideal": 720 }
            }),
            tls_enabled: false,
            tls_cert_path: "cert.pem".to_string(),
            tls_key_path: "key.pem".to_string(),
            admin_api_enabled: false,
            admin_api_key: "admin-secret-key".to_string(),
        }
    });

    let config_arc = Arc::new(config);

    // Start STUN server
    let stun_config = config_arc.clone();
    tokio::task::spawn(async move {
        let stun_addr: SocketAddr = stun_config.stun_addr.parse().expect("Invalid STUN address");
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
    let turn_config = config_arc.clone();
    tokio::task::spawn(async move {
        let turn_addr: SocketAddr = turn_config.turn_addr.parse().expect("Invalid TURN address");
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
    
    let config_api = config_arc.clone();
    let config_route = warp::path("api")
        .and(warp::path("config"))
        .and(warp::get())
        .and(warp::header::optional::<String>("host"))
        .map(move |_host: Option<String>| {
            let mut config_response = config_api.as_ref().clone();
            
            // If we can determine the server IP, replace localhost in ice_servers
            if let Some(local_ip) = network::get_local_ip() {
                let local_ip_str = local_ip.to_string();
                
                // Update ice_servers to use the actual IP instead of localhost
                for ice_server in &mut config_response.ice_servers {
                    ice_server.urls = ice_server.urls.iter().map(|url| {
                        url.replace("localhost", &local_ip_str)
                           .replace("127.0.0.1", &local_ip_str)
                    }).collect();
                }
            }
            
            warp::reply::json(&config_response)
        });

    // Admin API Routes
    let room_manager_status = room_manager.clone();
    let config_status = config_arc.clone();
    let admin_status = warp::path!("api" / "admin" / "status")
        .and(warp::get())
        .and(warp::header::optional::<String>("x-admin-api-key"))
        .and(warp::any().map(move || config_status.clone()))
        .and(warp::any().map(move || boot_time))
        .and(warp::any().map(move || room_manager_status.clone()))
        .and_then(|key_opt: Option<String>, config: Arc<Config>, boot_time: std::time::Instant, room_manager: Arc<RwLock<RoomManager>>| async move {
            if let Err(reply) = check_admin_auth(key_opt.as_deref(), &config) {
                return Ok::<_, warp::Rejection>(reply);
            }
            
            let uptime = std::time::Instant::now().duration_since(boot_time).as_secs();
            let manager = room_manager.read().await;
            let total_rooms = manager.rooms.len();
            let total_connections = manager.get_total_connections();
            
            Ok::<_, warp::Rejection>(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({
                    "uptime_seconds": uptime,
                    "total_rooms": total_rooms,
                    "total_connections": total_connections
                })),
                warp::http::StatusCode::OK
            ))
        });

    let room_manager_rooms = room_manager.clone();
    let config_rooms = config_arc.clone();
    let admin_rooms = warp::path!("api" / "admin" / "rooms")
        .and(warp::get())
        .and(warp::header::optional::<String>("x-admin-api-key"))
        .and(warp::any().map(move || config_rooms.clone()))
        .and(warp::any().map(move || room_manager_rooms.clone()))
        .and_then(|key_opt: Option<String>, config: Arc<Config>, room_manager: Arc<RwLock<RoomManager>>| async move {
            if let Err(reply) = check_admin_auth(key_opt.as_deref(), &config) {
                return Ok::<_, warp::Rejection>(reply);
            }
            
            let manager = room_manager.read().await;
            let rooms_status = manager.get_rooms_status();
            
            Ok::<_, warp::Rejection>(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({
                    "rooms": rooms_status
                })),
                warp::http::StatusCode::OK
            ))
        });

    let room_manager_del = room_manager.clone();
    let config_del = config_arc.clone();
    let clients_del = clients.clone();
    let admin_delete_room = warp::path!("api" / "admin" / "rooms" / String)
        .and(warp::delete())
        .and(warp::header::optional::<String>("x-admin-api-key"))
        .and(warp::any().map(move || config_del.clone()))
        .and(warp::any().map(move || room_manager_del.clone()))
        .and(warp::any().map(move || clients_del.clone()))
        .and_then(|room_id: String, key_opt: Option<String>, config: Arc<Config>, room_manager: Arc<RwLock<RoomManager>>, clients: Clients| async move {
            if let Err(reply) = check_admin_auth(key_opt.as_deref(), &config) {
                return Ok::<_, warp::Rejection>(reply);
            }
            
            let mut manager = room_manager.write().await;
            if let Some(connections) = manager.delete_room(&room_id) {
                let clients_guard = clients.read().await;
                for cid in connections {
                    if let Some(tx) = clients_guard.get(&cid) {
                        let leave_msg = SignalingMessage {
                            message_type: signaling::SignalingMessageType::Error,
                            connection_id: Some(cid.clone()),
                            sender_id: None,
                            offer_id: None,
                            data: Some(serde_json::json!({
                                "error": "Room closed by administrator"
                            })),
                            is_sender: None,
                        };
                        if let Ok(text) = serde_json::to_string(&leave_msg) {
                            let _ = tx.send(Message::text(text));
                        }
                    }
                }
                
                drop(clients_guard);
                
                Ok::<_, warp::Rejection>(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"message": "Room closed successfully"})),
                    warp::http::StatusCode::OK
                ))
            } else {
                Ok::<_, warp::Rejection>(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": "Room not found"})),
                    warp::http::StatusCode::NOT_FOUND
                ))
            }
        });

    let api_routes = create_room_route
        .or(get_room_route)
        .or(config_route)
        .or(admin_status)
        .or(admin_rooms)
        .or(admin_delete_room);
    
    // Static file serving for HTML clients
    let static_files = warp::fs::dir("static");
    
    // Combine all routes
    let routes = ws_route
        .or(api_routes)
        .or(static_files)
        .with(warp::cors().allow_any_origin().allow_methods(vec!["GET", "POST"]));
    
    let addr: SocketAddr = config_arc.signaling_addr.parse().expect("Invalid signaling address");
    
    if config_arc.tls_enabled {
        // Generate certificates if they don't exist
        if !std::path::Path::new(&config_arc.tls_cert_path).exists() || !std::path::Path::new(&config_arc.tls_key_path).exists() {
            info!("Generating self-signed certificate...");
            let subject_alt_names = get_all_local_ips();
            info!("Certificate will be valid for: {:?}", subject_alt_names);
            let cert = generate_simple_self_signed(subject_alt_names)?;
            fs::write(&config_arc.tls_cert_path, cert.serialize_pem()?)?;
            fs::write(&config_arc.tls_key_path, cert.serialize_private_key_pem())?;
            info!("Certificate generated: {} and {}", config_arc.tls_cert_path, config_arc.tls_key_path);
        }

        info!("Server listening on https://{}", addr);
        info!("Web client UI is available at:");
        info!("  - Sender (Camera): https://localhost:8080/sender.html");
        info!("  - Viewer (Monitor): https://localhost:8080/viewer.html");
        
        if let Some(local_ip) = network::get_local_ip() {
            info!("  - Sender (LAN): https://{}:8080/sender.html", local_ip);
            info!("  - Viewer (LAN): https://{}:8080/viewer.html", local_ip);
            info!("Note: You may need to accept the self-signed certificate warning on your mobile device.");
            if config_arc.admin_api_enabled {
                info!("  - Admin API: https://{}:8080/api/admin/status", local_ip);
            }
        } else if config_arc.admin_api_enabled {
            info!("  - Admin API: https://localhost:8080/api/admin/status");
        }
        
        warp::serve(routes)
            .tls()
            .cert_path(&config_arc.tls_cert_path)
            .key_path(&config_arc.tls_key_path)
            .run(addr)
            .await;
    } else {
        info!("Server listening on http://{}", addr);
        info!("Web client UI is available at:");
        info!("  - Sender (Camera): http://localhost:8080/sender.html");
        info!("  - Viewer (Monitor): http://localhost:8080/viewer.html");
        
        if let Some(local_ip) = network::get_local_ip() {
            info!("  - Sender (LAN): http://{}:8080/sender.html", local_ip);
            info!("  - Viewer (LAN): http://{}:8080/viewer.html", local_ip);
            if config_arc.admin_api_enabled {
                info!("  - Admin API: http://{}:8080/api/admin/status", local_ip);
            }
        } else if config_arc.admin_api_enabled {
            info!("  - Admin API: http://localhost:8080/api/admin/status");
        }
        warp::serve(routes)
            .run(addr)
            .await;
    }
    
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

fn check_admin_auth(
    key_opt: Option<&str>,
    config: &Config,
) -> Result<(), warp::reply::WithStatus<warp::reply::Json>> {
    if !config.admin_api_enabled {
        return Err(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": "Forbidden"})),
            warp::http::StatusCode::FORBIDDEN,
        ));
    }
    if key_opt != Some(&config.admin_api_key) {
        return Err(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": "Unauthorized"})),
            warp::http::StatusCode::UNAUTHORIZED,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use warp::http::StatusCode;

    fn test_config(enabled: bool) -> Arc<Config> {
        Arc::new(Config {
            signaling_addr: "0.0.0.0:8080".to_string(),
            stun_addr: "0.0.0.0:3478".to_string(),
            turn_addr: "0.0.0.0:3479".to_string(),
            ice_servers: vec![],
            video_constraints: serde_json::json!({}),
            tls_enabled: false,
            tls_cert_path: "cert.pem".to_string(),
            tls_key_path: "key.pem".to_string(),
            admin_api_enabled: enabled,
            admin_api_key: "test-token".to_string(),
        })
    }

    #[tokio::test]
    async fn test_admin_api_disabled() {
        let config = test_config(false);

        let route = warp::path!("api" / "admin" / "status")
            .and(warp::get())
            .and(warp::header::optional::<String>("x-admin-api-key"))
            .and(warp::any().map(move || config.clone()))
            .and_then(|key_opt: Option<String>, config: Arc<Config>| async move {
                if let Err(reply) = check_admin_auth(key_opt.as_deref(), &config) {
                    return Ok::<_, warp::Rejection>(reply);
                }
                Ok::<_, warp::Rejection>(warp::reply::with_status(warp::reply::json(&serde_json::json!({})), StatusCode::OK))
            });

        let resp = warp::test::request()
            .method("GET")
            .path("/api/admin/status")
            .header("x-admin-api-key", "test-token")
            .reply(&route)
            .await;

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_admin_api_unauthorized() {
        let config = test_config(true);

        let route = warp::path!("api" / "admin" / "status")
            .and(warp::get())
            .and(warp::header::optional::<String>("x-admin-api-key"))
            .and(warp::any().map(move || config.clone()))
            .and_then(|key_opt: Option<String>, config: Arc<Config>| async move {
                if let Err(reply) = check_admin_auth(key_opt.as_deref(), &config) {
                    return Ok::<_, warp::Rejection>(reply);
                }
                Ok::<_, warp::Rejection>(warp::reply::with_status(warp::reply::json(&serde_json::json!({})), StatusCode::OK))
            });

        let resp = warp::test::request()
            .method("GET")
            .path("/api/admin/status")
            .header("x-admin-api-key", "wrong-token")
            .reply(&route)
            .await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_admin_api_authorized() {
        let config = test_config(true);

        let route = warp::path!("api" / "admin" / "status")
            .and(warp::get())
            .and(warp::header::optional::<String>("x-admin-api-key"))
            .and(warp::any().map(move || config.clone()))
            .and_then(|key_opt: Option<String>, config: Arc<Config>| async move {
                if let Err(reply) = check_admin_auth(key_opt.as_deref(), &config) {
                    return Ok::<_, warp::Rejection>(reply);
                }
                Ok::<_, warp::Rejection>(warp::reply::with_status(warp::reply::json(&serde_json::json!({"ok": true})), StatusCode::OK))
            });

        let resp = warp::test::request()
            .method("GET")
            .path("/api/admin/status")
            .header("x-admin-api-key", "test-token")
            .reply(&route)
            .await;

        assert_eq!(resp.status(), StatusCode::OK);
    }
}
