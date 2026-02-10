//! Network connection info provider for GET /v1/network/connection_info
//!
//! Builds NetworkConnectionInfo from FeagiConfig and IOSystem (PNS) snapshots.

use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::v1::{
    ConnectionInfoApi, ConnectionInfoBluetooth, ConnectionInfoShm, ConnectionInfoStreamStatus,
    ConnectionInfoUdp, ConnectionInfoWebSocket, ConnectionInfoWebSocketEndpoints,
    ConnectionInfoWebSocketPorts, ConnectionInfoZmq, ConnectionInfoZmqEndpoints,
    ConnectionInfoZmqPorts, NetworkConnectionInfo,
};
use feagi_io::IOSystem;
use std::sync::Arc;

/// Provider for GET /v1/network/connection_info
pub struct FeagiNetworkConnectionInfoProvider {
    pub api_host: String,
    pub api_port: u16,
    pub pns: Arc<IOSystem>,
    pub viz_transport_policy: String,
}

impl NetworkConnectionInfoProvider for FeagiNetworkConnectionInfoProvider {
    fn get(&self) -> NetworkConnectionInfo {
        let snapshot = self.pns.get_connection_config_snapshot();
        let stream_status = self.pns.get_stream_status();

        let base_url = format!("http://{}:{}", self.api_host, self.api_port);
        let swagger_url = format!("{}/swagger-ui/", base_url);

        NetworkConnectionInfo {
            api: ConnectionInfoApi {
                enabled: true,
                base_url: base_url.clone(),
                host: self.api_host.clone(),
                port: self.api_port,
                swagger_url,
            },
            zmq: ConnectionInfoZmq {
                enabled: true,
                host: snapshot.zmq_host.clone(),
                ports: ConnectionInfoZmqPorts {
                    registration: snapshot.zmq_registration_port,
                    sensory: snapshot.zmq_sensory_port,
                    motor: snapshot.zmq_motor_port,
                    visualization: snapshot.zmq_viz_port,
                    api_control: snapshot.zmq_api_control_port,
                },
                endpoints: ConnectionInfoZmqEndpoints {
                    registration: format!(
                        "tcp://{}:{}",
                        snapshot.zmq_host, snapshot.zmq_registration_port
                    ),
                    sensory: format!(
                        "tcp://{}:{}",
                        snapshot.zmq_host, snapshot.zmq_sensory_port
                    ),
                    motor: format!("tcp://{}:{}", snapshot.zmq_host, snapshot.zmq_motor_port),
                    visualization: format!(
                        "tcp://{}:{}",
                        snapshot.zmq_host, snapshot.zmq_viz_port
                    ),
                },
            },
            websocket: ConnectionInfoWebSocket {
                enabled: snapshot.ws_enabled,
                host: snapshot.ws_host.clone(),
                ports: ConnectionInfoWebSocketPorts {
                    registration: snapshot.ws_registration_port,
                    sensory: snapshot.ws_sensory_port,
                    motor: snapshot.ws_motor_port,
                    visualization: snapshot.ws_viz_port,
                    rest_api: snapshot.ws_rest_api_port,
                },
                endpoints: ConnectionInfoWebSocketEndpoints {
                    registration: format!(
                        "ws://{}:{}",
                        snapshot.ws_host, snapshot.ws_registration_port
                    ),
                    sensory: format!(
                        "ws://{}:{}",
                        snapshot.ws_host, snapshot.ws_sensory_port
                    ),
                    motor: format!("ws://{}:{}", snapshot.ws_host, snapshot.ws_motor_port),
                    visualization: format!(
                        "ws://{}:{}",
                        snapshot.ws_host, snapshot.ws_viz_port
                    ),
                },
            },
            shm: ConnectionInfoShm {
                enabled: !matches!(self.viz_transport_policy.as_str(), "websocket"),
                base_path: snapshot.shm_base_path.clone(),
                policy: self.viz_transport_policy.clone(),
                note: "Actual paths (e.g. /tmp/feagi-shm-{agent_id}-sensory.bin) are allocated per-agent at registration"
                    .to_string(),
            },
            udp: ConnectionInfoUdp {
                enabled: false,
                visualization: None,
                sensory: None,
                note: "Placeholder for future UDP transport support".to_string(),
            },
            bluetooth: ConnectionInfoBluetooth {
                enabled: false,
                relay_port: None,
                note: "Placeholder for future use. Bluetooth relay is provided by feagi-desktop for embodied controllers, not by FEAGI server"
                    .to_string(),
            },
            stream_status: ConnectionInfoStreamStatus {
                zmq_control_started: stream_status.zmq_control_started,
                zmq_data_streams_started: stream_status.zmq_data_streams_started,
                websocket_started: stream_status.websocket_started,
                note: "Data streams start when genome is loaded and agents with matching capabilities are registered"
                    .to_string(),
            },
        }
    }
}
