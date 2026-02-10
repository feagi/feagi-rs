//! Network Connection Info Provider for feagi-rs
//!
//! Builds NetworkConnectionInfo from FeagiConfig and FeagiAgentHandler snapshots.

use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::v1::NetworkConnectionInfo;
use feagi_agent::server::FeagiAgentHandler;
use std::sync::{Arc, Mutex};

pub struct FeagiNetworkConnectionInfoProvider {
    pub api_host: String,
    pub api_port: u16,
    pub agent_handler: Arc<Mutex<FeagiAgentHandler>>,
    pub viz_transport_policy: String,
}

impl NetworkConnectionInfoProvider for FeagiNetworkConnectionInfoProvider {
    fn get(&self) -> NetworkConnectionInfo {
        // TODO: Extract actual endpoint information from agent handler
        // For now, return placeholder info - needs proper implementation
        NetworkConnectionInfo {
            api: feagi_api::v1::ConnectionInfoApi {
                enabled: true,
                base_url: format!("http://{}:{}", self.api_host, self.api_port),
                host: self.api_host.clone(),
                port: self.api_port,
                swagger_url: format!("http://{}:{}/swagger-ui/", self.api_host, self.api_port),
            },
            zmq: feagi_api::v1::ConnectionInfoZmq {
                enabled: true,
                host: self.api_host.clone(),
                ports: feagi_api::v1::ConnectionInfoZmqPorts {
                    registration: 5550,
                    sensory: 5551,
                    motor: 5552,
                    visualization: 5553,
                    api_control: 5554,
                },
                endpoints: feagi_api::v1::ConnectionInfoZmqEndpoints {
                    registration: format!("tcp://{}:5550", self.api_host),
                    sensory: format!("tcp://{}:5551", self.api_host),
                    motor: format!("tcp://{}:5552", self.api_host),
                    visualization: format!("tcp://{}:5553", self.api_host),
                },
            },
            websocket: feagi_api::v1::ConnectionInfoWebSocket {
                enabled: true,
                host: self.api_host.clone(),
                ports: feagi_api::v1::ConnectionInfoWebSocketPorts {
                    registration: 9050,
                    sensory: 9051,
                    motor: 9052,
                    visualization: 9053,
                    rest_api: 9054,
                },
                endpoints: feagi_api::v1::ConnectionInfoWebSocketEndpoints {
                    registration: format!("ws://{}:9050", self.api_host),
                    sensory: format!("ws://{}:9051", self.api_host),
                    motor: format!("ws://{}:9052", self.api_host),
                    visualization: format!("ws://{}:9053", self.api_host),
                },
            },
            shm: feagi_api::v1::ConnectionInfoShm {
                enabled: false,
                base_path: "/tmp/feagi-shm".to_string(),
                policy: self.viz_transport_policy.clone(),
                note: "SHM support pending".to_string(),
            },
            udp: feagi_api::v1::ConnectionInfoUdp {
                enabled: false,
                visualization: None,
                sensory: None,
                note: "UDP support pending".to_string(),
            },
            bluetooth: feagi_api::v1::ConnectionInfoBluetooth {
                enabled: false,
                relay_port: None,
                note: "Bluetooth support pending".to_string(),
            },
            stream_status: feagi_api::v1::ConnectionInfoStreamStatus {
                zmq_control_started: false,
                zmq_data_streams_started: false,
                websocket_started: false,
                note: "Stream status pending".to_string(),
            },
        }
    }
}
