// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for the NPU state WebSocket.
//!
//! These subscribe with `feagi-io`'s own WebSocket client rather than a raw WebSocket library, so
//! the frames are checked through the same path a real FEAGI agent uses.

use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use feagi::{FeagiConfig, FeagiInstance, WebSocketConfig};
use feagi_io::protocol_implementations::websocket::websocket_std::FeagiWebSocketClientSubscriberProperties;
use feagi_io::traits_and_enums::client::{FeagiClientSubscriber, FeagiClientSubscriberProperties};
use feagi_io::traits_and_enums::shared::FeagiEndpointState;
use feagi_serialization::{FeagiByteContainer, FeagiJSON};

const RECEIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Asks the OS for a free port, then releases it so the publisher can claim it.
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read the ephemeral port")
        .port()
}

/// Starts an instance broadcasting on a free port. Returns the instance and the stream URL.
fn start_instance(websocket_hz: u64) -> (Arc<FeagiInstance>, String) {
    let config = FeagiConfig {
        api_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        api_port: 0,
        burst_hz: 100,
        websocket: Some(WebSocketConfig::new("127.0.0.1", free_port(), websocket_hz)),
    };

    let instance = Arc::new(FeagiInstance::new(config));
    let status = instance
        .start_websocket()
        .expect("websocket publisher binds")
        .expect("websocket is configured");

    (instance, status.advertised_address)
}

fn subscribe(url: &str) -> Box<dyn FeagiClientSubscriber> {
    let properties =
        FeagiWebSocketClientSubscriberProperties::new(url).expect("subscriber url is valid");
    let mut subscriber = properties.as_boxed_client_subscriber();
    subscriber
        .request_connect()
        .expect("subscriber connects to the publisher");
    subscriber
}

/// Polls until a frame arrives, mirroring how a poll-based consumer drives the client.
fn next_frame(subscriber: &mut dyn FeagiClientSubscriber) -> Vec<u8> {
    let deadline = Instant::now() + RECEIVE_TIMEOUT;
    while Instant::now() < deadline {
        // Cloned so the borrow of `subscriber` ends before the data is consumed.
        match subscriber.poll().clone() {
            FeagiEndpointState::ActiveHasData => {
                return subscriber
                    .consume_retrieved_data()
                    .expect("frame bytes are readable")
                    .to_vec();
            }
            FeagiEndpointState::Errored(err) => panic!("subscriber errored: {err}"),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    panic!("no frame received within {RECEIVE_TIMEOUT:?}");
}

/// Unpacks the FEAGI byte container and returns the JSON payload it carries.
fn decode_state_frame(bytes: &[u8]) -> serde_json::Value {
    let mut container = FeagiByteContainer::new_empty();
    container
        .try_write_data_by_copy_and_verify(bytes)
        .expect("frame is a valid FEAGI byte container");

    let mut payload = FeagiJSON::new_empty();
    let found = container
        .try_update_struct_from_first_found_struct_of_type(&mut payload)
        .expect("JSON structure is readable");
    assert!(found, "frame carried no JSON structure");

    payload.borrow_json_value().clone()
}

#[test]
fn broadcasts_npu_state_over_websocket() {
    let (instance, url) = start_instance(50);

    let id = feagi::npu::parse_cortical_id("cust0100").expect("cortical id is valid");
    instance
        .npu()
        .add_cortical_area(id, 3, 3, 2, 2)
        .expect("cortical area is added");
    instance.npu().step_once();

    let mut subscriber = subscribe(&url);
    let payload = decode_state_frame(&next_frame(subscriber.as_mut()));

    assert_eq!(payload["kind"], "npu_state");
    assert_eq!(payload["burst_frequency_hz"], 100);
    assert!(payload["burst_count"].as_u64().expect("burst count") >= 1);

    let areas = payload["cortical_areas"]
        .as_array()
        .expect("cortical area list");
    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0]["cortical_id"], "cust0100");
    assert_eq!(areas[0]["cortical_dimensions"], json!([3, 3, 2]));
    assert_eq!(areas[0]["neurons_per_voxel"], 2);
    assert_eq!(areas[0]["neuron_count"], 36);

    let status = instance.websocket_status().expect("websocket status");
    assert!(status.running);
    assert!(status.frames_published > 0);
    assert_eq!(status.last_error, None);
}

#[test]
fn keeps_streaming_as_bursts_advance() {
    let (instance, url) = start_instance(50);
    let mut subscriber = subscribe(&url);

    instance.start_burst_engine();

    // The first frame may have been queued before the engine started, so walk forward until one
    // shows the counter has moved. That a later frame differs from an earlier one is what proves
    // this is a live stream rather than a single snapshot repeated.
    let first = decode_state_frame(&next_frame(subscriber.as_mut()));
    let first_count = first["burst_count"].as_u64().expect("burst count");

    let deadline = Instant::now() + RECEIVE_TIMEOUT;
    let advanced = loop {
        let frame = decode_state_frame(&next_frame(subscriber.as_mut()));
        if frame["burst_count"].as_u64().expect("burst count") > first_count {
            break Some(frame);
        }
        if Instant::now() >= deadline {
            break None;
        }
    };

    let advanced =
        advanced.unwrap_or_else(|| panic!("burst count never advanced past {first_count}"));
    assert_eq!(advanced["burst_running"], true);

    instance.stop_burst_engine();
}

#[test]
fn stops_publishing_once_the_transport_is_stopped() {
    let (instance, url) = start_instance(50);
    let mut subscriber = subscribe(&url);
    next_frame(subscriber.as_mut());

    assert!(instance.stop_websocket());
    assert!(instance.websocket_status().is_none());

    // Reconnecting must fail now that the listener is closed.
    let properties =
        FeagiWebSocketClientSubscriberProperties::new(&url).expect("subscriber url is valid");
    let mut reconnect = properties.as_boxed_client_subscriber();
    let connected = reconnect.request_connect().is_ok()
        && !matches!(reconnect.poll(), FeagiEndpointState::Errored(_));
    assert!(!connected, "publisher accepted a connection after stopping");
}

#[tokio::test]
async fn reports_the_stream_endpoint_over_http() {
    let (instance, url) = start_instance(20);

    let listener = instance.bind().await.expect("bind the api port");
    let address = listener.local_addr().expect("api address");
    let serve_instance = Arc::clone(&instance);
    tokio::spawn(async move { serve_instance.serve_on(listener).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let body: serde_json::Value = reqwest::get(format!("http://{address}/v1/system/websocket"))
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");

    assert_eq!(body["enabled"], true);
    assert_eq!(body["running"], true);
    assert_eq!(body["url"], url);
    assert_eq!(body["publish_hz"], 20);
}

#[tokio::test]
async fn reports_the_transport_as_disabled_when_unconfigured() {
    let config = FeagiConfig {
        api_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        api_port: 0,
        burst_hz: 100,
        websocket: None,
    };
    let instance = Arc::new(FeagiInstance::new(config));
    assert!(instance
        .start_websocket()
        .expect("no-op start succeeds")
        .is_none());

    let listener = instance.bind().await.expect("bind the api port");
    let address = listener.local_addr().expect("api address");
    let serve_instance = Arc::clone(&instance);
    tokio::spawn(async move { serve_instance.serve_on(listener).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let body: serde_json::Value = reqwest::get(format!("http://{address}/v1/system/websocket"))
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");

    assert_eq!(body["enabled"], false);
}
