// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Subscribes to a running server's NPU state stream and prints each frame.
//!
//! ```text
//! cargo run --example subscribe_npu_state -- ws://127.0.0.1:9050
//! ```
//!
//! Ask the server where to connect if you don't know the port:
//! `GET /v1/system/websocket` returns it as `url`.

use std::time::Duration;

use feagi_io::protocol_implementations::websocket::websocket_std::FeagiWebSocketClientSubscriberProperties;
use feagi_io::traits_and_enums::client::FeagiClientSubscriberProperties;
use feagi_io::traits_and_enums::shared::FeagiEndpointState;
use feagi_serialization::{FeagiByteContainer, FeagiJSON};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("ws://127.0.0.1:{}", feagi::DEFAULT_WEBSOCKET_PORT));
    let frames_wanted: usize = std::env::args()
        .nth(2)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(5);

    let properties = FeagiWebSocketClientSubscriberProperties::new(&url)?;
    let mut subscriber = properties.as_boxed_client_subscriber();
    subscriber.request_connect()?;
    println!("subscribed to {url}, waiting for {frames_wanted} frames");

    let mut container = FeagiByteContainer::new_empty();
    let mut payload = FeagiJSON::new_empty();
    let mut received = 0;

    while received < frames_wanted {
        // Cloned so the borrow of `subscriber` ends before the data is consumed.
        match subscriber.poll().clone() {
            FeagiEndpointState::ActiveHasData => {
                let bytes = subscriber.consume_retrieved_data()?;
                if container.try_write_data_by_copy_and_verify(bytes).is_err() {
                    eprintln!("frame was not a valid FEAGI byte container");
                    continue;
                }
                match container.try_update_struct_from_first_found_struct_of_type(&mut payload) {
                    Ok(true) => {
                        received += 1;
                        println!("frame {received}: {payload}");
                    }
                    Ok(false) => eprintln!("frame carried no JSON structure"),
                    Err(()) => eprintln!("frame's JSON structure could not be read"),
                }
            }
            FeagiEndpointState::Errored(err) => return Err(Box::new(err)),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }

    subscriber.request_disconnect()?;
    Ok(())
}
