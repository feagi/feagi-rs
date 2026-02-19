//! Network Connection Info Provider for feagi-rs
//!
//! Builds NetworkConnectionInfo from FeagiConfig and FeagiAgentHandler snapshots.

use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::v1::NetworkConnectionInfo;
use feagi_agent::server::FeagiAgentHandler;
use std::sync::{Arc, Mutex};

pub struct FeagiNetworkConnectionInfoProvider {
    pub api_advertised_host: String,
    pub api_port: u16,
    pub agent_handler: Arc<Mutex<FeagiAgentHandler>>,
    pub viz_transport_policy: String,
    pub zmq_enabled: bool,
    pub zmq_registration_advertised_host: String,
    pub zmq_advertised_host: String,
    pub zmq_registration_port: u16,
    pub zmq_sensory_port: u16,
    pub zmq_motor_port: u16,
    pub zmq_visualization_port: u16,
    pub zmq_api_control_port: u16,
    pub websocket_enabled: bool,
    pub websocket_advertised_host: String,
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
                base_url: format!("http://{}:{}", self.api_advertised_host, self.api_port),
                host: self.api_advertised_host.clone(),
                port: self.api_port,
                swagger_url: format!(
                    "http://{}:{}/swagger-ui/",
                    self.api_advertised_host, self.api_port
                ),
            },
            zmq: feagi_api::v1::ConnectionInfoZmq {
                enabled: self.zmq_enabled,
                host: self.zmq_advertised_host.clone(),
                ports: feagi_api::v1::ConnectionInfoZmqPorts {
                    registration: self.zmq_registration_port,
                    sensory: self.zmq_sensory_port,
                    motor: self.zmq_motor_port,
                    visualization: self.zmq_visualization_port,
                    api_control: self.zmq_api_control_port,
                },
                endpoints: feagi_api::v1::ConnectionInfoZmqEndpoints {
                    registration: format!(
                        "tcp://{}:{}",
                        self.zmq_registration_advertised_host, self.zmq_registration_port
                    ),
                    sensory: format!("tcp://{}:{}", self.zmq_advertised_host, self.zmq_sensory_port),
                    motor: format!("tcp://{}:{}", self.zmq_advertised_host, self.zmq_motor_port),
                    visualization: format!(
                        "tcp://{}:{}",
                        self.zmq_advertised_host, self.zmq_visualization_port
                    ),
                },
            },
            websocket: feagi_api::v1::ConnectionInfoWebSocket {
                enabled: self.websocket_enabled,
                host: self.websocket_advertised_host.clone(),
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
                        self.websocket_advertised_host, self.websocket_registration_port
                    ),
                    sensory: format!(
                        "ws://{}:{}",
                        self.websocket_advertised_host, self.websocket_sensory_port
                    ),
                    motor: format!(
                        "ws://{}:{}",
                        self.websocket_advertised_host, self.websocket_motor_port
                    ),
                    visualization: format!(
                        "ws://{}:{}",
                        self.websocket_advertised_host, self.websocket_visualization_port
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

#[cfg(test)]
mod tests {
    use super::*;
    use feagi_agent::server::auth::DummyAuth;

    #[test]
    fn connection_info_uses_advertised_hosts() {
        let agent_handler = Arc::new(Mutex::new(FeagiAgentHandler::new(Box::new(
            DummyAuth {},
        ))));

        let provider = FeagiNetworkConnectionInfoProvider {
            api_advertised_host: "127.0.0.1".to_string(),
            api_port: 8000,
            agent_handler,
            viz_transport_policy: "websocket".to_string(),
            zmq_enabled: true,
            zmq_registration_advertised_host: "127.0.0.1".to_string(),
            zmq_advertised_host: "127.0.0.1".to_string(),
            zmq_registration_port: 30001,
            zmq_sensory_port: 5555,
            zmq_motor_port: 5564,
            zmq_visualization_port: 5562,
            zmq_api_control_port: 5563,
            websocket_enabled: true,
            websocket_advertised_host: "127.0.0.1".to_string(),
            websocket_registration_port: 9053,
            websocket_sensory_port: 9051,
            websocket_motor_port: 9052,
            websocket_visualization_port: 9050,
            websocket_rest_api_port: 9054,
        };

        let info = provider.get();
        assert_eq!(info.api.host, "127.0.0.1");
        assert_eq!(info.websocket.host, "127.0.0.1");
        assert_eq!(info.websocket.endpoints.visualization, "ws://127.0.0.1:9050");
        assert_eq!(info.zmq.endpoints.registration, "tcp://127.0.0.1:30001");
        assert_eq!(info.zmq.ports.api_control, 5563);
    }
}
