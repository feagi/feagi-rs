// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! HTTP API for the FEAGI server.
//!
//! The route shape follows the previous `feagi-api` crate so existing clients keep working where
//! the underlying capability still exists. Routes whose backing subsystem was removed in the NPU
//! rewrite are registered as explicit `501 Not Implemented` stubs rather than 404s, so clients can
//! tell "gone for now" apart from "wrong URL".

use std::sync::Arc;

use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};
use tracing::warn;

use crate::npu::{parse_cortical_id, CorticalAreaRecord, NpuError, NpuHandle};
use crate::ws::WebSocketBroadcaster;

/// Shared state handed to every handler.
#[derive(Clone)]
pub struct ApiState {
    pub npu: NpuHandle,
    /// `None` when the server runs without the WebSocket transport.
    pub websocket: Option<Arc<WebSocketBroadcaster>>,
}

impl ApiState {
    pub fn new(npu: NpuHandle, websocket: Option<Arc<WebSocketBroadcaster>>) -> Self {
        Self { npu, websocket }
    }
}

/// An error rendered as a JSON body, so clients never get a bare status code.
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<NpuError> for ApiError {
    fn from(err: NpuError) -> Self {
        let status = match err {
            NpuError::DuplicateCorticalArea(_) => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        ApiError::new(status, err.to_string())
    }
}

type ApiResult<T> = Result<T, ApiError>;

//region Cortical areas

/// Body for `POST /v1/cortical_area/cortical_area`.
///
/// `cortical_dimensions` is the historical field name; `dimensions` is accepted as an alias.
#[derive(Debug, Deserialize)]
pub struct CreateCorticalAreaRequest {
    pub cortical_id: String,
    #[serde(alias = "dimensions")]
    pub cortical_dimensions: [u64; 3],
    /// Neurons per voxel. Defaults to 1.
    #[serde(default)]
    pub neurons_per_voxel: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CorticalAreaResponse {
    pub cortical_id: String,
    pub cortical_id_base64: String,
    pub cortical_dimensions: [u64; 3],
    pub neurons_per_voxel: u64,
    pub neuron_count: u64,
}

impl From<CorticalAreaRecord> for CorticalAreaResponse {
    fn from(record: CorticalAreaRecord) -> Self {
        let [x, y, z, d] = record.dimensions;
        Self {
            cortical_id: record.id_ascii(),
            cortical_id_base64: record.id.as_base_64(),
            cortical_dimensions: [x, y, z],
            neurons_per_voxel: d,
            neuron_count: record.neuron_count,
        }
    }
}

/// `POST /v1/cortical_area/cortical_area` — creates a cortical area in the NPU.
async fn post_cortical_area(
    State(state): State<ApiState>,
    Json(request): Json<CreateCorticalAreaRequest>,
) -> ApiResult<(StatusCode, Json<CorticalAreaResponse>)> {
    let id = parse_cortical_id(&request.cortical_id)?;
    let [x, y, z] = request.cortical_dimensions;
    let density = request.neurons_per_voxel.unwrap_or(1);

    let record = state.npu.add_cortical_area(id, x, y, z, density)?;
    Ok((StatusCode::CREATED, Json(record.into())))
}

/// `GET /v1/cortical_area/cortical_area_id_list`
async fn get_cortical_area_id_list(State(state): State<ApiState>) -> Json<Vec<String>> {
    Json(
        state
            .npu
            .cortical_areas()
            .into_iter()
            .map(|area| area.id_ascii())
            .collect(),
    )
}

/// `GET /v1/cortical_area/cortical_area_list`
async fn get_cortical_area_list(State(state): State<ApiState>) -> Json<Vec<CorticalAreaResponse>> {
    Json(
        state
            .npu
            .cortical_areas()
            .into_iter()
            .map(CorticalAreaResponse::from)
            .collect(),
    )
}

/// `GET /v1/cortical_area/cortical_area/:cortical_id`
async fn get_cortical_area(
    State(state): State<ApiState>,
    Path(cortical_id): Path<String>,
) -> ApiResult<Json<CorticalAreaResponse>> {
    let id = parse_cortical_id(&cortical_id)?;
    state
        .npu
        .cortical_area(&id)
        .map(|record| Json(record.into()))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no cortical area '{cortical_id}'"),
            )
        })
}

//endregion

//region Burst engine

#[derive(Debug, Serialize)]
pub struct BurstStatusResponse {
    pub running: bool,
    pub burst_count: u64,
    pub burst_frequency_hz: u64,
    pub cortical_area_count: usize,
}

/// `GET /v1/burst_engine/status`
async fn get_burst_status(State(state): State<ApiState>) -> Json<BurstStatusResponse> {
    Json(BurstStatusResponse {
        running: state.npu.is_running(),
        burst_count: state.npu.burst_count(),
        burst_frequency_hz: state.npu.burst_hz(),
        cortical_area_count: state.npu.cortical_areas().len(),
    })
}

/// `POST /v1/burst_engine/start`
async fn post_burst_start(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let started = state.npu.start();
    Json(json!({
        "running": true,
        "changed": started,
        "burst_frequency_hz": state.npu.burst_hz(),
    }))
}

/// `POST /v1/burst_engine/stop`
async fn post_burst_stop(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let stopped = state.npu.stop();
    Json(json!({
        "running": false,
        "changed": stopped,
        "burst_count": state.npu.burst_count(),
    }))
}

/// `POST /v1/burst_engine/burst` — runs a single burst, for stepping a stopped engine.
async fn post_single_burst(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let burst_count = state.npu.step_once();
    Json(json!({ "burst_count": burst_count }))
}

#[derive(Debug, Deserialize)]
pub struct BurstFrequencyRequest {
    #[serde(alias = "burst_frequency", alias = "hz")]
    pub burst_frequency_hz: u64,
}

/// `PUT /v1/burst_engine/burst_frequency`
async fn put_burst_frequency(
    State(state): State<ApiState>,
    Json(request): Json<BurstFrequencyRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    state.npu.set_burst_hz(request.burst_frequency_hz)?;
    Ok(Json(json!({ "burst_frequency_hz": state.npu.burst_hz() })))
}

//endregion

//region System

/// `GET /v1/system/health_check`
async fn get_health_check(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let cortical_area_count = state.npu.cortical_areas().len();
    Json(json!({
        "burst_engine": state.npu.is_running(),
        "burst_count": state.npu.burst_count(),
        "cortical_area_count": cortical_area_count,
        "genome_availability": false,
        "brain_readiness": cortical_area_count > 0,
    }))
}

/// `GET /v1/system/version`
async fn get_version() -> Json<serde_json::Value> {
    Json(json!({
        "feagi_rs": env!("CARGO_PKG_VERSION"),
        "npu": "in-progress feagi-core rewrite (feagi-npu::dynamic_npu)",
    }))
}

/// `GET /v1/system/websocket` — where to subscribe for the NPU state stream, and how it is doing.
///
/// This is how a client discovers the stream endpoint rather than hardcoding a port.
async fn get_websocket_status(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let Some(status) = state.websocket.as_ref().map(|ws| ws.status()) else {
        return Json(json!({
            "enabled": false,
            "detail": "server started without the websocket transport",
        }));
    };

    Json(json!({
        "enabled": true,
        "running": status.running,
        "url": status.advertised_address,
        "bind_address": status.bind_address,
        "publish_hz": status.publish_hz,
        "frames_published": status.frames_published,
        "last_error": status.last_error,
        "payload": "FEAGI byte container holding a JSON npu_state object",
    }))
}

//endregion

//region Removed subsystems

/// Handler for routes whose subsystem was removed in the NPU rewrite.
///
/// Returns `501` with the route name so clients can distinguish a temporarily unavailable feature
/// from a typo in the URL.
async fn not_implemented(OriginalUri(uri): OriginalUri) -> Response {
    // `OriginalUri` rather than `Uri`, so nested routes report the full request path.
    let path = uri.path().to_string();
    warn!(target: "feagi-rs", "request to unimplemented endpoint {path}");
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "endpoint not available on the in-progress NPU",
            "path": path,
            "detail": "this subsystem has not been ported to the feagi-core NPU rewrite yet",
        })),
    )
        .into_response()
}

/// Route prefixes that existed on the previous server and are not yet ported.
const UNIMPLEMENTED_PREFIXES: &[&str] = &[
    "/agent",
    "/connectome",
    "/cortical_mapping",
    "/evolution",
    "/genome",
    "/input",
    "/insight",
    "/monitoring",
    "/morphology",
    "/network",
    "/neuroplasticity",
    "/output",
    "/physiology",
    "/region",
    "/simulation",
    "/training",
    "/visualization",
];

//endregion

/// Builds the `/v1` router.
fn create_v1_router() -> Router<ApiState> {
    let mut router = Router::new()
        .route("/cortical_area/cortical_area", post(post_cortical_area))
        .route(
            "/cortical_area/cortical_area/:cortical_id",
            get(get_cortical_area),
        )
        .route(
            "/cortical_area/cortical_area_id_list",
            get(get_cortical_area_id_list),
        )
        .route(
            "/cortical_area/cortical_area_list",
            get(get_cortical_area_list),
        )
        .route("/burst_engine/status", get(get_burst_status))
        .route("/burst_engine/start", post(post_burst_start))
        .route("/burst_engine/stop", post(post_burst_stop))
        .route("/burst_engine/burst", post(post_single_burst))
        .route("/burst_engine/burst_frequency", put(put_burst_frequency))
        .route("/system/health_check", get(get_health_check))
        .route("/system/version", get(get_version))
        .route("/system/websocket", get(get_websocket_status));

    for prefix in UNIMPLEMENTED_PREFIXES {
        router = router
            .route(prefix, any(not_implemented))
            .route(&format!("{prefix}/*rest"), any(not_implemented));
    }

    router
}

/// Builds the full application router.
pub fn create_http_server(state: ApiState) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Redirect::temporary("/v1/system/health_check") }),
        )
        .nest("/v1", create_v1_router())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}
