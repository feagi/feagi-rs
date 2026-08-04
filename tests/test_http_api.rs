// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests against a real server bound to an ephemeral port.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use feagi::{FeagiConfig, FeagiInstance};
use serde_json::json;

/// Starts a server on an OS-assigned port and returns its base URL.
async fn start_server() -> (Arc<FeagiInstance>, String) {
    let config = FeagiConfig {
        api_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        api_port: 0,
        burst_hz: 100,
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

#[tokio::test]
async fn adds_a_cortical_area_over_http() {
    let (instance, base) = start_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base}/v1/cortical_area/cortical_area"))
        .json(&json!({
            "cortical_id": "cust0001",
            "cortical_dimensions": [4, 4, 2],
            "neurons_per_voxel": 2,
        }))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);

    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["cortical_id"], "cust0001");
    assert_eq!(body["cortical_dimensions"], json!([4, 4, 2]));
    assert_eq!(body["neurons_per_voxel"], 2);
    assert_eq!(body["neuron_count"], 64);

    // The area is visible to the NPU handle, not just echoed back.
    assert_eq!(instance.npu().cortical_areas().len(), 1);

    let ids: Vec<String> = client
        .get(format!("{base}/v1/cortical_area/cortical_area_id_list"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    assert_eq!(ids, vec!["cust0001".to_string()]);
}

#[tokio::test]
async fn rejects_duplicate_and_malformed_cortical_areas() {
    let (_instance, base) = start_server().await;
    let client = reqwest::Client::new();

    let create = |id: &str, dims: serde_json::Value| {
        client
            .post(format!("{base}/v1/cortical_area/cortical_area"))
            .json(&json!({ "cortical_id": id, "cortical_dimensions": dims }))
            .send()
    };

    assert_eq!(
        create("cust0002", json!([2, 2, 2]))
            .await
            .expect("request sent")
            .status(),
        reqwest::StatusCode::CREATED
    );

    // Same ID twice.
    assert_eq!(
        create("cust0002", json!([2, 2, 2]))
            .await
            .expect("request sent")
            .status(),
        reqwest::StatusCode::CONFLICT
    );

    // 'z' is not a recognized cortical type prefix.
    assert_eq!(
        create("zzzz0001", json!([2, 2, 2]))
            .await
            .expect("request sent")
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // A zero axis would allocate no neurons.
    assert_eq!(
        create("cust0003", json!([2, 0, 2]))
            .await
            .expect("request sent")
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn accepts_base64_cortical_ids() {
    let (_instance, base) = start_server().await;
    let client = reqwest::Client::new();

    // Base64 of the raw bytes "cust0009".
    let encoded = "Y3VzdDAwMDk=";

    let response = client
        .post(format!("{base}/v1/cortical_area/cortical_area"))
        .json(&json!({ "cortical_id": encoded, "cortical_dimensions": [1, 1, 1] }))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["cortical_id"], "cust0009");
    assert_eq!(body["cortical_id_base64"], encoded);
}

#[tokio::test]
async fn runs_bursts_and_reports_status() {
    let (instance, base) = start_server().await;
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/v1/cortical_area/cortical_area"))
        .json(&json!({ "cortical_id": "cust0004", "cortical_dimensions": [3, 3, 3] }))
        .send()
        .await
        .expect("request sent");

    // Single-step the engine while it is stopped.
    let stepped: serde_json::Value = client
        .post(format!("{base}/v1/burst_engine/burst"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    assert_eq!(stepped["burst_count"], 1);

    // Now run the loop and confirm bursts accumulate on their own.
    client
        .post(format!("{base}/v1/burst_engine/start"))
        .send()
        .await
        .expect("request sent");

    tokio::time::sleep(Duration::from_millis(300)).await;

    let status: serde_json::Value = client
        .get(format!("{base}/v1/burst_engine/status"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");

    assert_eq!(status["running"], true);
    assert_eq!(status["cortical_area_count"], 1);
    let burst_count = status["burst_count"].as_u64().expect("burst count");
    assert!(
        burst_count > 1,
        "expected the burst loop to advance past the manual step, got {burst_count}"
    );

    client
        .post(format!("{base}/v1/burst_engine/stop"))
        .send()
        .await
        .expect("request sent");

    let settled = instance.npu().burst_count();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        instance.npu().burst_count(),
        settled,
        "burst count should stop advancing once the engine is stopped"
    );
}

#[tokio::test]
async fn reports_unported_subsystems_as_not_implemented() {
    let (_instance, base) = start_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base}/v1/genome/load"))
        .json(&json!({}))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_IMPLEMENTED);
    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["path"], "/v1/genome/load");
}
