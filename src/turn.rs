use std::net::{SocketAddr, UdpSocket};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket as TokioUdpSocket;
use log::{info, error, debug};
use byteorder::{BigEndian, ByteOrder};
use uuid::Uuid;

// TURN message types
const ALLOCATE_REQUEST: u16 = 0x0003;
const ALLOCATE_RESPONSE: u16 = 0x0103;
const ALLOCATE_ERROR_RESPONSE: u16 = 0x0113;
const SEND_INDICATION: u16 = 0x0016;
const DATA_INDICATION: u16 = 0x0117;

// TURN attribute types
const XOR_RELAYED_ADDRESS: u16 = 0x0016;
const LIFETIME: u16 = 0x000d;
const XOR_PEER_ADDRESS: u16 = 0x0012;
const DATA: u16 = 0x0013;

#[derive(Debug, Clone)]
pub struct TurnAllocation {
    pub id: String,
    pub client_addr: SocketAddr,
    pub relayed_addr: SocketAddr,
    pub peer_addr: Option<SocketAddr>,
    pub lifetime: std::time::Instant,
    pub permissions: HashMap<SocketAddr, std::time::Instant>,
}

pub struct TurnServer {
    socket: Arc<TokioUdpSocket>,
    allocations: Arc<Mutex<HashMap<String, TurnAllocation>>>,
    relay_ports: Arc<Mutex<HashMap<u16, String>>>, // port -> allocation_id
    next_relay_port: u16,
}

impl TurnServer {
    pub fn new(bind_addr: SocketAddr) -> std::io::Result<Self> {
        let socket = std::net::UdpSocket::bind(bind_addr)?;
        socket.set_nonblocking(true)?;
        let tokio_socket = TokioUdpSocket::from_std(socket)?;
        
        info!("TURN server listening on {}", bind_addr);
        
        Ok(Self {
            socket: Arc::new(tokio_socket),
            allocations: Arc::new(Mutex::new(HashMap::new())),
            relay_ports: Arc::new(Mutex::new(HashMap::new())),
            next_relay_port: 49152, // Start of dynamic port range
        })
    }
    
    pub async fn run(&mut self) -> std::io::Result<()> {
        let mut buf = [0u8; 2048];
        
        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, src_addr)) => {
                    let packet = &buf[..len];
                    
                    if let Some(response) = self.handle_turn_packet(packet, src_addr).await {
                        if let Err(e) = self.socket.send_to(&response, src_addr).await {
                            error!("Failed to send TURN response: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("TURN server error: {}", e);
                }
            }
        }
    }
    
    async fn handle_turn_packet(&mut self, packet: &[u8], src_addr: SocketAddr) -> Option<Vec<u8>> {
        if packet.len() < 20 {
            debug!("Packet too short for TURN message");
            return None;
        }
        
        let msg_type = BigEndian::read_u16(&packet[0..2]);
        let msg_len = BigEndian::read_u16(&packet[2..4]);
        
        // Verify packet length
        if packet.len() != 20 + msg_len as usize {
            debug!("TURN packet length mismatch");
            return None;
        }
        
        match msg_type {
            ALLOCATE_REQUEST => {
                debug!("TURN allocate request from {}", src_addr);
                Some(self.create_allocate_response(packet, src_addr).await)
            }
            SEND_INDICATION => {
                debug!("TURN send indication from {}", src_addr);
                self.handle_send_indication(packet, src_addr).await;
                None
            }
            _ => {
                debug!("Unsupported TURN message type: 0x{:04x}", msg_type);
                Some(self.create_error_response(packet, 400, "Bad Request"))
            }
        }
    }
    
    async fn create_allocate_response(&mut self, request: &[u8], client_addr: SocketAddr) -> Vec<u8> {
        let allocation_id = Uuid::new_v4().to_string();
        let relayed_port = self.get_next_relay_port();
        let relayed_addr = SocketAddr::new(client_addr.ip(), relayed_port);
        
        // Create allocation
        let allocation = TurnAllocation {
            id: allocation_id.clone(),
            client_addr,
            relayed_addr,
            peer_addr: None,
            lifetime: std::time::Instant::now() + std::time::Duration::from_secs(600), // 10 minutes
            permissions: HashMap::new(),
        };
        
        // Store allocation
        {
            let mut allocations = self.allocations.lock().unwrap();
            allocations.insert(allocation_id.clone(), allocation);
        }
        
        {
            let mut relay_ports = self.relay_ports.lock().unwrap();
            relay_ports.insert(relayed_port, allocation_id.clone());
        }
        
        info!("Created TURN allocation {} for {} -> {}", allocation_id, client_addr, relayed_addr);
        
        // Build response
        let mut response = Vec::new();
        
        // Message header
        response.extend_from_slice(&ALLOCATE_RESPONSE.to_be_bytes());
        response.extend_from_slice(&0u16.to_be_bytes()); // Length (placeholder)
        response.extend_from_slice(&request[4..20]); // Copy magic cookie and transaction ID
        
        // XOR-RELAYED-ADDRESS attribute
        let attr_type = XOR_RELAYED_ADDRESS;
        let attr_len = 8u16;
        
        response.extend_from_slice(&attr_type.to_be_bytes());
        response.extend_from_slice(&attr_len.to_be_bytes());
        response.push(0x00); // Reserved
        response.push(0x01); // IPv4 family
        
        let ip = relayed_addr.ip();
        let port = relayed_addr.port() ^ 0x2112; // XOR with magic cookie
        
        response.extend_from_slice(&port.to_be_bytes());
        
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                let octets = ipv4.octets();
                for octet in octets {
                    response.push(octet ^ 0x21); // XOR with magic cookie bytes
                }
            }
            std::net::IpAddr::V6(_) => {
                response.extend_from_slice(&[0; 16]);
            }
        }
        
        // LIFETIME attribute (600 seconds)
        let lifetime_attr = LIFETIME;
        let lifetime_len = 4u16;
        response.extend_from_slice(&lifetime_attr.to_be_bytes());
        response.extend_from_slice(&lifetime_len.to_be_bytes());
        response.extend_from_slice(&600u32.to_be_bytes());
        
        // Update message length
        let total_len = response.len() - 20;
        response[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        
        response
    }
    
    async fn handle_send_indication(&self, packet: &[u8], src_addr: SocketAddr) {
        // Parse XOR-PEER-ADDRESS and DATA attributes
        let mut peer_addr = None;
        let mut data = None;
        
        let mut pos = 20; // Skip header
        while pos + 4 <= packet.len() {
            let attr_type = BigEndian::read_u16(&packet[pos..pos+2]);
            let attr_len = BigEndian::read_u16(&packet[pos+2..pos+4]);
            pos += 4;
            
            if pos + attr_len as usize > packet.len() {
                break;
            }
            
            match attr_type {
                XOR_PEER_ADDRESS => {
                    if attr_len >= 8 {
                        let port = BigEndian::read_u16(&packet[pos+2..pos+4]) ^ 0x2112;
                        let ip_bytes = &packet[pos+4..pos+8];
                        let mut octets = [0u8; 4];
                        for (i, &byte) in ip_bytes.iter().enumerate() {
                            octets[i] = byte ^ 0x21;
                        }
                        let ip = std::net::Ipv4Addr::from(octets);
                        peer_addr = Some(SocketAddr::new(std::net::IpAddr::V4(ip), port));
                    }
                }
                DATA => {
                    data = Some(&packet[pos..pos+attr_len as usize]);
                }
                _ => {}
            }
            
            pos += (attr_len as usize + 3) & !3; // Round up to 4-byte boundary
        }
        
        if let (Some(peer), Some(data_bytes)) = (peer_addr, data) {
            debug!("Relaying data from {} to {}", src_addr, peer);
            
            // In a real implementation, you would forward this data to the peer
            // For now, we just log it
            info!("TURN relay: {} -> {} ({} bytes)", src_addr, peer, data_bytes.len());
        }
    }
    
    fn create_error_response(&self, request: &[u8], code: u16, reason: &str) -> Vec<u8> {
        let mut response = Vec::new();
        
        // Message header
        response.extend_from_slice(&ALLOCATE_ERROR_RESPONSE.to_be_bytes());
        response.extend_from_slice(&0u16.to_be_bytes()); // Length (placeholder)
        response.extend_from_slice(&request[4..20]); // Copy magic cookie and transaction ID
        
        // ERROR-CODE attribute
        let error_class = code / 100;
        let error_number = code % 100;
        let reason_bytes = reason.as_bytes();
        let attr_len = 4 + reason_bytes.len() as u16;
        
        response.extend_from_slice(&0u16.to_be_bytes()); // ERROR-CODE attribute type
        response.extend_from_slice(&attr_len.to_be_bytes());
        response.extend_from_slice(&0u16.to_be_bytes());
        response.push((error_class / 100) as u8);
        response.push((error_class % 100) as u8);
        response.extend_from_slice(reason_bytes);
        
        // Update message length
        let total_len = response.len() - 20;
        response[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        
        response
    }
    
    fn get_next_relay_port(&mut self) -> u16 {
        let port = self.next_relay_port;
        if self.next_relay_port == 65535 {
            self.next_relay_port = 49152; // Wrap around
        } else {
            self.next_relay_port += 1;
        }
        port
    }
    
    pub fn get_local_address(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}
