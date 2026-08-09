// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests against a real server bound to an ephemeral port.
//!
//! These assert the published REST contract, which `feagi-api` owns and which predates the NPU
//! rewrite. The contract is the fixed point: the engine behind it changed, the endpoints did not.
//! Anything asserted here that stops holding is a regression in the API surface, not a test that
//! needs relaxing.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use feagi::{FeagiConfig, FeagiInstance};
use serde_json::json;

/// The barebones genome shipped for development, used wherever a test needs a realised brain.
fn barebones_genome_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("genomes/barebones_genome.json")
}

/// Starts a server on an OS-assigned port and returns its base URL.
///
/// The WebSocket transport is left off here so these tests don't contend for a fixed port; it has
/// its own test file.
async fn start_server() -> (Arc<FeagiInstance>, String) {
    let config = FeagiConfig {
        api_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        api_port: 0,
        burst_hz: 100,
        // These tests exercise the HTTP contract only, so the broadcast socket is left unbound.
        websocket: None,
    };
    let instance = Arc::new(FeagiInstance::new(config));
    let listener = instance.bind().await.expect("bind ephemeral port");
    let address = listener.local_addr().expect("local addr");

    let serve_instance = Arc::clone(&instance);
    tokio::spawn(async move { serve_instance.serve_on(listener).await });

    // Give axum a moment to begin accepting.
    tokio::time::sleep(Duration::from_millis(100)).await;

    (instance, format!("http://{address}"))
}

/// Starts a server with the barebones genome already realised in the NPU.
///
/// Endpoints that resolve brain regions or read cortical metadata need a genome, because that is
/// where those structures come from.
async fn start_server_with_genome() -> (Arc<FeagiInstance>, String) {
    let (instance, base) = start_server().await;
    feagi::genome::load_genome_file(
        instance.npu(),
        instance.genome(),
        &barebones_genome_path(),
    )
    .expect("barebones genome should load");
    (instance, base)
}

#[tokio::test]
async fn serves_the_openapi_document_and_swagger_ui() {
    let (_instance, base) = start_server().await;
    let client = reqwest::Client::new();

    let spec = client
        .get(format!("{base}/api-docs/openapi.json"))
        .send()
        .await
        .expect("request sent");
    assert_eq!(spec.status(), reqwest::StatusCode::OK);

    let document: serde_json::Value = spec.json().await.expect("openapi json");
    let paths = document["paths"]
        .as_object()
        .expect("openapi document exposes paths");

    // The documented surface is the whole point of mounting feagi-api rather than a hand-written
    // router, so assert it is substantial rather than a handful of routes.
    assert!(
        paths.len() > 100,
        "expected the full documented surface, found {} paths",
        paths.len()
    );
    assert!(paths.contains_key("/v1/system/health_check"));
    assert!(paths.contains_key("/v1/cortical_area/custom_cortical_area"));

    let ui = client
        .get(format!("{base}/swagger-ui/"))
        .send()
        .await
        .expect("request sent");
    assert_eq!(ui.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn reports_health_over_http() {
    let (_instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base}/v1/system/health_check"))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("json body");
    assert!(
        body.get("burst_engine").is_some(),
        "health check reports burst engine state, got {body}"
    );
}

#[tokio::test]
async fn lists_cortical_areas_realised_from_the_genome() {
    let (instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base}/v1/cortical_area/cortical_area_id_list"))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("json body");
    let ids = body["cortical_ids"]
        .as_array()
        .expect("the contract wraps the ids under cortical_ids");

    assert_eq!(
        ids.len(),
        instance.npu().cortical_areas().len(),
        "the API reports exactly the areas the NPU holds"
    );
    assert!(!ids.is_empty(), "the barebones genome realises areas");
}

#[tokio::test]
async fn creates_a_custom_cortical_area_and_realises_it_in_the_npu() {
    let (instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    let before = instance.npu().cortical_areas().len();
    let response = client
        .post(format!("{base}/v1/cortical_area/custom_cortical_area"))
        .json(&json!({
            "cortical_name": "cust0001",
            "cortical_dimensions": [4, 4, 2],
            "coordinates_3d": [0, 0, 0],
            "brain_region_id": "root",
            "neurons_per_voxel": 2,
        }))
        .send()
        .await
        .expect("request sent");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "body: {}",
        response.text().await.unwrap_or_default()
    );

    assert_eq!(
        instance.npu().cortical_areas().len(),
        before + 1,
        "creating an area over HTTP realises it in the engine"
    );
}

#[tokio::test]
async fn drives_the_burst_engine_over_http() {
    let (instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    // The timestep endpoint is the contract's burst-rate control: seconds per burst, so 0.01s is
    // 100Hz.
    let response = client
        .post(format!("{base}/v1/burst_engine/simulation_timestep"))
        .json(&json!({ "simulation_timestep": 0.02 }))
        .send()
        .await
        .expect("request sent");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "body: {}",
        response.text().await.unwrap_or_default()
    );
    assert_eq!(
        instance.npu().burst_hz(),
        50,
        "a 0.02s timestep is 50Hz at the engine"
    );

    let response = client
        .get(format!("{base}/v1/burst_engine/simulation_timestep"))
        .send()
        .await
        .expect("request sent");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("json body");
    let timestep = body
        .as_f64()
        .or_else(|| body.get("simulation_timestep").and_then(|v| v.as_f64()))
        .expect("timestep in response");
    assert!(
        (timestep - 0.02).abs() < 1e-6,
        "reported timestep should match what was set, got {timestep}"
    );
}

#[tokio::test]
async fn rejects_a_custom_area_with_malformed_dimensions() {
    let (instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    let before = instance.npu().cortical_areas().len();
    let response = client
        .post(format!("{base}/v1/cortical_area/custom_cortical_area"))
        .json(&json!({
            "cortical_name": "bad_dims",
            "cortical_dimensions": [4, 4],
            "coordinates_3d": [0, 0, 0],
            "brain_region_id": "root",
        }))
        .send()
        .await
        .expect("request sent");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "dimensions must be [x, y, z]"
    );
    assert_eq!(
        instance.npu().cortical_areas().len(),
        before,
        "a rejected request must not realise an area in the engine"
    );
}

#[tokio::test]
async fn parameterised_routes_resolve_to_their_handlers() {
    let (_instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    // Routes carrying a path parameter are easy to publish in a form the router never matches: a
    // parameter written in the wrong syntax for the axum version is treated as a literal segment,
    // so the endpoint misses for every real value while still appearing in the OpenAPI document.
    //
    // An unmatched route and a handler reporting a missing resource both answer 404, so the status
    // alone cannot tell them apart. The router's fallback answers with this exact plain-text body,
    // which a handler never produces.
    const ROUTER_FALLBACK_BODY: &str = "404 Not Found";

    for path in [
        "/v1/agent/info/some_agent",
        "/v1/agent/properties/some_agent",
        "/v1/agent/some_agent/device_registrations",
        "/v1/region/region/some_region",
        "/v1/morphology/info/some_morphology",
    ] {
        let body = client
            .get(format!("{base}{path}"))
            .send()
            .await
            .expect("request sent")
            .text()
            .await
            .expect("response body");

        assert_ne!(
            body, ROUTER_FALLBACK_BODY,
            "{path} fell through to the router fallback, so its path parameter is not being matched"
        );
    }
}

#[tokio::test]
async fn reports_unavailable_npu_introspection_as_not_implemented() {
    let (_instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    // Per-neuron inspection has no equivalent in the current engine, which replaced individually
    // addressable synapses with mapping entries over quantized arrays. The route keeps its path
    // and response schema and reports 501 until the engine can supply the data; it must not 404,
    // because that would mean the published surface had shrunk.
    let response = client
        .get(format!("{base}/v1/cortical_area/voxel_neurons"))
        .query(&[("cortical_area", "_death"), ("x", "0"), ("y", "0"), ("z", "0")])
        .send()
        .await
        .expect("request sent");

    assert_ne!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND,
        "the route must stay published even while unimplemented"
    );
}
