use mediasoup::worker::{Worker, WorkerLogLevel, WorkerLogTag, WorkerSettings};
use mediasoup::router::{Router, RouterOptions};
use mediasoup::rtp_parameters::{RtpCapabilities, RtpCodecCapability};
use mediasoup::transport::{Transport, TransportListenInfo, TransportListenIp};
use mediasoup::producer::Producer;
use mediasoup::consumer::Consumer;
use mediasoup::data_structures::{ListenInfo, Protocol};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfuRoom {
    pub id: String,
    pub router_id: String,
    pub producer_id: Option<String>,
    pub consumer_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SfuConnection {
    pub id: String,
    pub is_sender: bool,
    pub transport_id: Option<String>,
    pub producer_id: Option<String>,
    pub consumer_ids: Vec<String>,
}

pub struct SfuManager {
    worker: Arc<Worker>,
    router: Arc<Router>,
    rooms: Arc<RwLock<HashMap<String, SfuRoom>>>,
    connections: Arc<RwLock<HashMap<String, SfuConnection>>>,
    producers: Arc<RwLock<HashMap<String, Arc<Producer>>>>,
    consumers: Arc<RwLock<HashMap<String, Arc<Consumer>>>>,
}

impl SfuManager {
    pub async fn new() -> Result<Self> {
        // Create worker
        let worker_settings = WorkerSettings {
            log_level: WorkerLogLevel::Debug,
            log_tags: vec![
                WorkerLogTag::Info,
                WorkerLogTag::Ice,
                WorkerLogTag::Dtls,
                WorkerLogTag::Rtp,
                WorkerLogTag::Srtp,
                WorkerLogTag::Rtcp,
                WorkerLogTag::Rtx,
                WorkerLogTag::Bwe,
                WorkerLogTag::Scores,
                WorkerLogTag::Simulcast,
                WorkerLogTag::Svc,
            ],
            rtc_min_port: 10000,
            rtc_max_port: 20000,
            dtls_certificate_file: None,
            dtls_private_key_file: None,
            libwebrtc_field_trials: None,
        };

        let worker = Arc::new(Worker::new(worker_settings, None).await?);

        // Create router with media codecs
        let router_options = RouterOptions {
            media_codecs: vec![
                RtpCodecCapability::Audio {
                    mime_type: "audio/opus".to_string(),
                    preferred_payload_type: Some(100),
                    clock_rate: 48000,
                    channels: Some(2),
                    parameters: Default::default(),
                },
                RtpCodecCapability::Video {
                    mime_type: "video/VP8".to_string(),
                    preferred_payload_type: Some(101),
                    clock_rate: 90000,
                    parameters: Default::default(),
                },
                RtpCodecCapability::Video {
                    mime_type: "video/H264".to_string(),
                    preferred_payload_type: Some(102),
                    clock_rate: 90000,
                    parameters: Default::default(),
                },
            ],
            ..Default::default()
        };

        let router = Arc::new(worker.create_router(router_options).await?);

        Ok(Self {
            worker,
            router,
            rooms: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            producers: Arc::new(RwLock::new(HashMap::new())),
            consumers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn create_room(&self, room_id: String) -> Result<()> {
        let room = SfuRoom {
            id: room_id.clone(),
            router_id: self.router.id().to_string(),
            producer_id: None,
            consumer_ids: Vec::new(),
        };

        let mut rooms = self.rooms.write().await;
        rooms.insert(room_id, room);
        Ok(())
    }

    pub async fn create_transport(&self, connection_id: String) -> Result<Transport> {
        // Create WebRTC transport
        let transport_options = mediasoup::transport::WebRtcTransportOptions {
            listen_ips: vec![
                TransportListenIp {
                    ip: "0.0.0.0".to_string(),
                    announced_ip: None,
                },
            ],
            enable_udp: true,
            enable_tcp: false,
            prefer_udp: true,
            initial_available_outgoing_bitrate: 600000,
            ..Default::default()
        };

        let transport = self.router.create_webrtc_transport(transport_options).await?;
        Ok(transport)
    }

    pub async fn add_connection(&self, room_id: &str, connection_id: String, is_sender: bool) -> Result<()> {
        let connection = SfuConnection {
            id: connection_id.clone(),
            is_sender,
            transport_id: None,
            producer_id: None,
            consumer_ids: Vec::new(),
        };

        let mut connections = self.connections.write().await;
        connections.insert(connection_id, connection);
        Ok(())
    }

    pub async fn create_producer(&self, connection_id: &str, transport: &Transport, rtp_parameters: &str) -> Result<String> {
        let producer_id = Uuid::new_v4().to_string();
        
        // Parse RTP parameters and create producer
        // This is simplified - in real implementation you'd parse the JSON rtp_parameters
        let producer_options = mediasoup::producer::ProducerOptions {
            kind: mediasoup::rtp_parameters::MediaKind::Video,
            rtp_parameters: Default::default(), // Parse from rtp_parameters
            ..Default::default()
        };

        let producer = Arc::new(transport.produce(producer_options).await?);
        
        let mut producers = self.producers.write().await;
        producers.insert(producer_id.clone(), producer);

        // Update connection with producer_id
        let mut connections = self.connections.write().await;
        if let Some(connection) = connections.get_mut(connection_id) {
            connection.producer_id = Some(producer_id.clone());
        }

        Ok(producer_id)
    }

    pub async fn create_consumer(&self, connection_id: &str, transport: &Transport, producer_id: &str) -> Result<String> {
        let consumer_id = Uuid::new_v4().to_string();
        
        // Get producer
        let producers = self.producers.read().await;
        if let Some(producer) = producers.get(producer_id) {
            let consumer_options = mediasoup::consumer::ConsumerOptions {
                producer_id: producer.id(),
                rtp_capabilities: self.router.rtp_capabilities().clone(),
                ..Default::default()
            };

            let consumer = Arc::new(transport.consume(consumer_options).await?);
            
            let mut consumers = self.consumers.write().await;
            consumers.insert(consumer_id.clone(), consumer);

            // Update connection with consumer_id
            let mut connections = self.connections.write().await;
            if let Some(connection) = connections.get_mut(connection_id) {
                connection.consumer_ids.push(consumer_id.clone());
            }

            Ok(consumer_id)
        } else {
            Err(anyhow::anyhow!("Producer not found: {}", producer_id))
        }
    }

    pub async fn get_router_rtp_capabilities(&self) -> RtpCapabilities {
        self.router.rtp_capabilities().clone()
    }

    pub async fn remove_connection(&self, connection_id: &str) -> Result<()> {
        let mut connections = self.connections.write().await;
        connections.remove(connection_id);
        Ok(())
    }

    pub async fn get_producer_ids_for_room(&self, _room_id: &str) -> Vec<String> {
        let producers = self.producers.read().await;
        producers.keys().cloned().collect()
    }
}
