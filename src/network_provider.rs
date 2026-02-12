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
    pub zmq_enabled: bool,
    pub zmq_registration_port: u16,
    pub zmq_sensory_port: u16,
    pub zmq_motor_port: u16,
    pub zmq_visualization_port: u16,
    pub websocket_enabled: bool,
    pub websocket_registration_port: u16,
    pub websocket_sensory_port: u16,
    pub websocket_motor_port: u16,
    pub websocket_visualization_port: u16,
    pub websocket_rest_api_port: u16,
}

impl NetworkConnectionInfoProvider for FeagiNetworkConnectionInfoProvider {
    fn get(&self) -> NetworkConnectionInfo {
        NetworkConnectionInfo {
            api: feagi_api::v1::ConnectionInfoApi {
                enabled: true,
                base_url: format!("http://{}:{}", self.api_host, self.api_port),
                host: self.api_host.clone(),
                port: self.api_port,
                swagger_url: format!("http://{}:{}/swagger-ui/", self.api_host, self.api_port),
            },
            zmq: feagi_api::v1::ConnectionInfoZmq {
                enabled: self.zmq_enabled,
                host: self.api_host.clone(),
                ports: feagi_api::v1::ConnectionInfoZmqPorts {
                    registration: self.zmq_registration_port,
                    sensory: self.zmq_sensory_port,
                    motor: self.zmq_motor_port,
                    visualization: self.zmq_visualization_port,
                    api_control: 5554,
                },
                endpoints: feagi_api::v1::ConnectionInfoZmqEndpoints {
                    registration: format!("tcp://{}:{}", self.api_host, self.zmq_registration_port),
                    sensory: format!("tcp://{}:{}", self.api_host, self.zmq_sensory_port),
                    motor: format!("tcp://{}:{}", self.api_host, self.zmq_motor_port),
                    visualization: format!("tcp://{}:{}", self.api_host, self.zmq_visualization_port),
                },
            },
            websocket: feagi_api::v1::ConnectionInfoWebSocket {
                enabled: self.websocket_enabled,
                host: self.api_host.clone(),
                ports: feagi_api::v1::ConnectionInfoWebSocketPorts {
                    registration: self.websocket_registration_port,
                    sensory: self.websocket_sensory_port,
                    motor: self.websocket_motor_port,
                    visualization: self.websocket_visualization_port,
                    rest_api: self.websocket_rest_api_port,
                },
                endpoints: feagi_api::v1::ConnectionInfoWebSocketEndpoints {
                    registration: format!(
                        "ws://{}:{}",
                        self.api_host, self.websocket_registration_port
                    ),
                    sensory: format!("ws://{}:{}", self.api_host, self.websocket_sensory_port),
                    motor: format!("ws://{}:{}", self.api_host, self.websocket_motor_port),
                    visualization: format!(
                        "ws://{}:{}",
                        self.api_host, self.websocket_visualization_port
                    ),
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
