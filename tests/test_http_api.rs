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
/// Only the HTTP surface is started: agent registration binds a fixed port from the configuration,
/// and it has its own test file.
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

/// Starts a server with the barebones genome already realised in the NPU.
///
/// Endpoints that resolve brain regions or read cortical metadata need a genome, because that is
/// where those structures come from.
async fn start_server_with_genome() -> (Arc<FeagiInstance>, String) {
    start_server_loading(&barebones_genome_path()).await
}

/// Starts a server with the given genome realised in the NPU.
///
/// Separate from [`start_server_with_genome`] because the barebones genome is flat and declares no
/// brain regions, so tests covering the region tree need a genome that does.
async fn start_server_loading(genome: &std::path::Path) -> (Arc<FeagiInstance>, String) {
    let (instance, base) = start_server().await;
    feagi::genome::load_genome_file(instance.npu(), instance.genome(), genome)
        .unwrap_or_else(|e| panic!("genome '{}' should load: {e}", genome.display()));
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
async fn uploads_the_barebones_genome_and_realises_it_in_the_npu() {
    let (instance, base) = start_server().await;
    let client = reqwest::Client::new();

    assert!(
        instance.npu().cortical_areas().is_empty(),
        "the server starts without a genome"
    );

    let response = client
        .post(format!("{base}/v1/genome/upload/barebones"))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["success"], json!(true));

    let reported_areas = body["cortical_area_count"]
        .as_u64()
        .expect("the response counts areas");
    assert!(reported_areas > 0, "the barebones genome contributes areas");
    assert_eq!(
        instance.npu().cortical_areas().len() as u64,
        reported_areas,
        "every area the response reports should be realised in the NPU"
    );
}

/// Loading a genome sets the burst frequency to the reciprocal of the genome's timestep, so a
/// placeholder timestep of zero would drive the engine at an unrepresentable rate.
#[tokio::test]
async fn a_genome_upload_leaves_the_engine_at_the_genomes_own_timestep() {
    let (_instance, base) = start_server().await;
    let client = reqwest::Client::new();

    let upload = client
        .post(format!("{base}/v1/genome/upload/barebones"))
        .send()
        .await
        .expect("request sent");
    assert_eq!(upload.status(), reqwest::StatusCode::OK);

    let response = client
        .get(format!("{base}/v1/burst_engine/simulation_timestep"))
        .send()
        .await
        .expect("request sent");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let timestep: f64 = response
        .json()
        .await
        .expect("the engine reports a timestep");

    assert!(
        timestep.is_finite() && timestep > 0.0,
        "the engine should run at the genome's timestep rather than a placeholder: got {timestep}"
    );
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

/// The morphology endpoints read the genome's morphology registry. Before that read was
/// implemented they answered 500, so this asserts both the status and that the payload actually
/// carries the genome's morphologies rather than an empty map.
#[tokio::test]
async fn lists_the_morphologies_carried_by_the_genome() {
    let (_instance, base) = start_server_with_genome().await;
    let client = reqwest::Client::new();

    let body: serde_json::Value = client
        .get(format!("{base}/v1/morphology/morphology_list"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");

    let names = body["morphology_list"]
        .as_array()
        .expect("morphology_list is an array");
    assert!(
        !names.is_empty(),
        "the barebones genome defines morphologies, so the list must not be empty"
    );

    let mut sorted = names.to_vec();
    sorted.sort_by_key(|value| value.as_str().unwrap_or_default().to_string());
    assert_eq!(names, &sorted, "the contract promises alphabetical order");

    // Each listed morphology must also be described, which exercises the type and parameter
    // conversion rather than just the key set.
    let described: serde_json::Value = client
        .get(format!("{base}/v1/morphology/morphologies"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");

    let first = names[0].as_str().expect("morphology name is a string");
    assert!(
        described.get(first).is_some(),
        "morphology '{first}' was listed but not described"
    );
}

/// Brain regions come from the genome, and each region's child list is reconstructed from the
/// parent link the genome records on the children. This covers both the listing and that
/// reconstruction.
#[tokio::test]
async fn describes_brain_regions_and_their_hierarchy() {
    let genome = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("genomes/memory_genome.json");
    let (_instance, base) = start_server_loading(&genome).await;
    let client = reqwest::Client::new();

    let ids: Vec<String> = client
        .get(format!("{base}/v1/region/regions"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    assert_eq!(
        ids.len(),
        2,
        "the memory genome declares two brain regions, got {ids:?}"
    );

    let titles: std::collections::HashMap<String, String> = client
        .get(format!("{base}/v1/region/region_titles"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    for id in &ids {
        assert!(
            titles.contains_key(id),
            "region '{id}' was listed but has no title"
        );
    }
    assert!(
        titles.values().any(|title| title == "Root Brain Region"),
        "region names must come from the genome, got {titles:?}"
    );

    // A region that claims a child must be named as that child's parent, and vice versa. This is
    // the invariant the reconstruction has to preserve, since the genome stores only one side.
    let mut children_of = std::collections::HashMap::new();
    let mut parent_of = std::collections::HashMap::new();
    for id in &ids {
        let detail: serde_json::Value = client
            .get(format!("{base}/v1/region/region/{id}"))
            .send()
            .await
            .expect("request sent")
            .json()
            .await
            .expect("json body");

        // The contract names these `regions` and `parent_region_id`, not after the DTO fields.
        children_of.insert(id.clone(), detail["regions"].clone());
        parent_of.insert(id.clone(), detail["parent_region_id"].clone());
    }

    for (parent, children) in &children_of {
        for child in children.as_array().into_iter().flatten() {
            let child = child.as_str().expect("child region id is a string");
            assert_eq!(
                parent_of.get(child).and_then(|value| value.as_str()),
                Some(parent.as_str()),
                "region '{parent}' claims '{child}' as a child, but '{child}' does not name it as parent"
            );
        }
    }
}

/// Creating a region writes it into the loaded genome, which is what the region reads and the
/// genome export both draw from. This walks create, read back, and delete.
#[tokio::test]
async fn creates_and_deletes_a_brain_region() {
    let genome = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("genomes/memory_genome.json");
    let (_instance, base) = start_server_loading(&genome).await;
    let client = reqwest::Client::new();

    let existing: Vec<String> = client
        .get(format!("{base}/v1/region/regions"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    let parent = existing.first().expect("genome has a region").clone();

    let created = client
        .post(format!("{base}/v1/region/region"))
        .json(&json!({
            "title": "Test Region",
            "parent_region_id": parent,
            "coordinates_2d": [0, 0],
            "coordinates_3d": [0, 0, 0],
        }))
        .send()
        .await
        .expect("request sent");
    assert!(
        created.status().is_success(),
        "creating a region failed: {}",
        created.text().await.unwrap_or_default()
    );

    // The new region must be visible to readers, carry its parent, and appear as the parent's
    // child, which is the derived side of the link.
    let titles: std::collections::HashMap<String, String> = client
        .get(format!("{base}/v1/region/region_titles"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    let new_id = titles
        .iter()
        .find(|(_, title)| title.as_str() == "Test Region")
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| panic!("the created region should be listed, got {titles:?}"));

    let parent_detail: serde_json::Value = client
        .get(format!("{base}/v1/region/region/{parent}"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    let children = parent_detail["regions"]
        .as_array()
        .expect("the detail response lists child regions under `regions`");
    assert!(
        children.iter().any(|child| child.as_str() == Some(&new_id)),
        "the parent should list the new region as a child, got {children:?}"
    );

    // The new region holds no cortical areas, so deleting it needs nothing from the engine.
    let deleted = client
        .delete(format!("{base}/v1/region/region"))
        .json(&json!({ "region_id": new_id }))
        .send()
        .await
        .expect("request sent");
    assert!(
        deleted.status().is_success(),
        "deleting an empty region failed: {}",
        deleted.text().await.unwrap_or_default()
    );

    let remaining: Vec<String> = client
        .get(format!("{base}/v1/region/regions"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    assert_eq!(
        remaining.len(),
        existing.len(),
        "the region count should be back to where it started"
    );
}

/// A server that has not loaded a genome must still answer the endpoints a monitor or a UI polls
/// on startup. Reporting "nothing loaded" as a 500 makes the server look broken during the window
/// before a genome arrives, which is exactly when these are polled hardest.
#[tokio::test]
async fn answers_read_endpoints_before_a_genome_is_loaded() {
    let (_instance, base) = start_server().await;
    let client = reqwest::Client::new();

    for path in [
        "/v1/system/health_check",
        "/v1/region/regions",
        "/v1/region/region_titles",
        "/v1/morphology/morphology_list",
        "/v1/cortical_area/cortical_area_id_list",
        "/v1/burst_engine/burst_counter",
    ] {
        let response = client
            .get(format!("{base}{path}"))
            .send()
            .await
            .expect("request sent");
        let status = response.status();
        assert!(
            status.is_success(),
            "{path} answered {status} with no genome loaded: {}",
            response.text().await.unwrap_or_default()
        );
    }

    // Health has to describe the empty state rather than merely not failing.
    let health: serde_json::Value = client
        .get(format!("{base}/v1/system/health_check"))
        .send()
        .await
        .expect("request sent")
        .json()
        .await
        .expect("json body");
    assert_eq!(health["cortical_area_count"], 0);
    assert_eq!(
        health["brain_readiness"], false,
        "a brain with no areas is not ready"
    );
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
        .query(&[
            ("cortical_area", "_death"),
            ("x", "0"),
            ("y", "0"),
            ("z", "0"),
        ])
        .send()
        .await
        .expect("request sent");

    assert_ne!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND,
        "the route must stay published even while unimplemented"
    );
}
