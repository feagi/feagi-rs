// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Exposes this crate's [`NpuHandle`] to the API services.
//!
//! `feagi-api` declares the engine capabilities its services need as
//! [`feagi_api::services::NpuAccess`] and leaves the implementation to whoever owns an engine.
//! This adapter supplies it, which is what lets the REST layer drive the running brain without
//! `feagi-api` depending on this crate.

use feagi_api::services::{NpuAccess, NpuCorticalArea};
use feagi_genomic_context::cortical_area::CorticalID;
use feagi_models::connectome_requests::connectome_request::ConnectomeRequest;

use crate::npu::{CorticalAreaRecord, NpuHandle};

/// Converts an engine record into the transport-neutral shape the services consume.
fn to_service_area(record: &CorticalAreaRecord) -> NpuCorticalArea {
    NpuCorticalArea {
        id: record.id,
        dimensions: record.dimensions,
        neuron_count: record.neuron_count,
    }
}

impl NpuAccess for NpuHandle {
    fn cortical_areas(&self) -> Vec<NpuCorticalArea> {
        NpuHandle::cortical_areas(self)
            .iter()
            .map(to_service_area)
            .collect()
    }

    fn cortical_area(&self, id: &CorticalID) -> Option<NpuCorticalArea> {
        NpuHandle::cortical_area(self, id).map(|record| to_service_area(&record))
    }

    fn add_cortical_area(
        &self,
        id: CorticalID,
        x: u64,
        y: u64,
        z: u64,
        density: u64,
    ) -> Result<NpuCorticalArea, String> {
        NpuHandle::add_cortical_area(self, id, x, y, z, density)
            .map(|record| to_service_area(&record))
            .map_err(|error| error.to_string())
    }

    fn submit_connectome_requests(&self, requests: Vec<ConnectomeRequest>) {
        NpuHandle::submit_connectome_requests(self, requests)
    }

    fn burst_count(&self) -> u64 {
        NpuHandle::burst_count(self)
    }

    fn burst_hz(&self) -> u64 {
        NpuHandle::burst_hz(self)
    }

    fn set_burst_hz(&self, hz: u64) -> Result<(), String> {
        NpuHandle::set_burst_hz(self, hz).map_err(|error| error.to_string())
    }

    fn is_running(&self) -> bool {
        NpuHandle::is_running(self)
    }

    fn start(&self) -> bool {
        NpuHandle::start(self)
    }

    fn stop(&self) -> bool {
        NpuHandle::stop(self)
    }

    fn step_once(&self) -> u64 {
        NpuHandle::step_once(self)
    }
}
