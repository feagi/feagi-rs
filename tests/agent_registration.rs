// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end coverage of the agent registration handshake.
//!
//! A real `feagi_agent` client is driven against the registration socket the server binds, so the
//! test exercises the same path the Brain Visualizer takes: connect, request registration for a
//! capability, and receive the endpoint that serves it. Nothing here is stubbed, because the
//! handshake itself is the subject under test.

use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use feagi::agent::{RegistrationConfig, RegistrationServer};
use feagi_agent::clients::{AgentRegistrationStatus, CommandControlAgent};
use feagi_agent::server::auth::DummyAuth;
use feagi_agent::server::FeagiAgentHandler;
use feagi_agent::{AgentCapabilities, AgentDescriptor, AuthToken};
use feagi_io::protocol_implementations::websocket::websocket_std::{
    FeagiWebSocketClientRequesterProperties, FeagiWebSocketClientSubscriberProperties,
    FeagiWebSocketServerPublisherProperties,
};
use feagi_io::traits_and_enums::client::FeagiClientSubscriberProperties;
use feagi_io::traits_and_enums::shared::TransportProtocolEndpoint;

/// How long a handshake may take before the test calls it a failure.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait between client polls while a handshake is in flight.
const CLIENT_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// An all-zero token, which is what `DummyAuth` accepts and what agents send when auth is off.
const TEST_AUTH_TOKEN_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

/// Reserves a port by binding it and letting the listener drop, so the server can take it next.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to reserve a local port");
    listener
        .local_addr()
        .expect("reserved listener has no address")
        .port()
}

fn test_descriptor() -> AgentDescriptor {
    AgentDescriptor::new("neuraville", "registration-test-agent", 1)
        .expect("test descriptor must be valid")
}

fn test_auth_token() -> AuthToken {
    AuthToken::from_base64(TEST_AUTH_TOKEN_B64).expect("test auth token must decode")
}

/// A handler holding one WebSocket visualization publisher, which is what a visualizer is granted.
fn handler_with_visualization_publisher(visualization_port: u16) -> Arc<Mutex<FeagiAgentHandler>> {
    let address = format!("127.0.0.1:{visualization_port}");
    let publisher = FeagiWebSocketServerPublisherProperties::new(&address, &address)
        .expect("failed to build visualization publisher properties");

    let mut handler = FeagiAgentHandler::new(Box::new(DummyAuth {}));
    handler.add_publisher_server(Box::new(publisher));
    Arc::new(Mutex::new(handler))
}

fn endpoint_contains_port(endpoint: &TransportProtocolEndpoint, port: u16) -> bool {
    format!("{endpoint:?}").contains(&port.to_string())
}

/// Registers a visualizer against `server` and returns the client and the endpoints it was granted.
///
/// The client is returned rather than dropped because dropping it closes the command-and-control
/// connection, which deregisters the agent and tears down what it was granted.
fn register_visualizer(
    registration_port: u16,
    server: &RegistrationServer,
) -> (
    CommandControlAgent,
    HashMap<AgentCapabilities, TransportProtocolEndpoint>,
) {
    let requester = FeagiWebSocketClientRequesterProperties::new(&format!(
        "ws://127.0.0.1:{registration_port}"
    ))
    .expect("failed to build client requester properties");
    let mut agent = CommandControlAgent::new(Box::new(requester));

    agent.request_connect().expect("client failed to connect");
    agent
        .request_registration(
            test_descriptor(),
            test_auth_token(),
            vec![AgentCapabilities::ReceiveNeuronVisualizations],
        )
        .expect("client failed to send its registration request");

    let started = Instant::now();
    let endpoints = loop {
        agent.poll_for_messages().expect("client poll failed");

        if let AgentRegistrationStatus::Registered(_, endpoints) = agent.registration_status() {
            break endpoints.clone();
        }

        assert!(
            started.elapsed() < HANDSHAKE_TIMEOUT,
            "registration did not complete within {HANDSHAKE_TIMEOUT:?}; server status: {:?}",
            server.status()
        );
        std::thread::sleep(CLIENT_POLL_INTERVAL);
    };

    (agent, endpoints)
}

#[test]
fn agent_registers_and_is_given_the_visualization_endpoint() {
    let registration_port = free_port();
    let visualization_port = free_port();
    let handler = handler_with_visualization_publisher(visualization_port);

    let address = format!("127.0.0.1:{registration_port}");
    let server = RegistrationServer::start(
        handler,
        RegistrationConfig {
            bind_address: address.clone(),
            advertised_address: address,
        },
    )
    .expect("registration server must start");

    let (_agent, endpoints) = register_visualizer(registration_port, &server);

    let visualization = endpoints
        .get(&AgentCapabilities::ReceiveNeuronVisualizations)
        .expect("a registered visualizer must be given a visualization endpoint");
    assert!(
        endpoint_contains_port(visualization, visualization_port),
        "visualization endpoint {visualization:?} does not point at port {visualization_port}"
    );

    let status = server.status();
    assert!(status.running, "server stopped while handling registration");
    assert_eq!(
        status.last_error, None,
        "server recorded an error during registration"
    );
}

/// Being handed a visualization endpoint is not the same as that endpoint being usable.
///
/// The publisher socket only accepts connections and answers the WebSocket upgrade while it is
/// being polled. When it was not, the Brain Visualizer registered successfully, connected at the
/// TCP level, and then sat in `STATE_CONNECTING` until it timed out. This drives a real subscriber
/// through the upgrade to keep that gap closed.
#[test]
fn the_granted_visualization_endpoint_accepts_a_subscriber() {
    let registration_port = free_port();
    let visualization_port = free_port();
    let handler = handler_with_visualization_publisher(visualization_port);

    let address = format!("127.0.0.1:{registration_port}");
    let server = RegistrationServer::start(
        handler,
        RegistrationConfig {
            bind_address: address.clone(),
            advertised_address: address,
        },
    )
    .expect("registration server must start");

    let (_agent, _endpoints) = register_visualizer(registration_port, &server);

    // The subscriber's connect blocks until the server answers the upgrade, so it runs on its own
    // thread: an unserviced publisher must fail this test by timing out rather than hanging it.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let properties = FeagiWebSocketClientSubscriberProperties::new(&format!(
            "ws://127.0.0.1:{visualization_port}"
        ))
        .expect("failed to build subscriber properties");
        let mut subscriber = properties.as_boxed_client_subscriber();
        let _ = sender.send(subscriber.request_connect().map_err(|err| err.to_string()));
    });

    let outcome = receiver
        .recv_timeout(HANDSHAKE_TIMEOUT)
        .unwrap_or_else(|_| {
            panic!(
                "visualization endpoint did not complete the WebSocket handshake within \
             {HANDSHAKE_TIMEOUT:?}; server status: {:?}",
                server.status()
            )
        });
    outcome.expect("subscriber was refused by the visualization endpoint");

    assert_eq!(
        server.status().last_error,
        None,
        "server recorded an error while the subscriber connected"
    );
}

#[test]
fn stopping_the_server_ends_polling() {
    let registration_port = free_port();
    let visualization_port = free_port();
    let handler = handler_with_visualization_publisher(visualization_port);

    let address = format!("127.0.0.1:{registration_port}");
    let server = RegistrationServer::start(
        handler,
        RegistrationConfig {
            bind_address: address.clone(),
            advertised_address: address,
        },
    )
    .expect("registration server must start");

    assert!(server.status().running);
    assert!(server.stop(), "first stop must report the running server");
    assert!(!server.status().running);
    assert!(!server.stop(), "a second stop must report nothing to stop");
}
