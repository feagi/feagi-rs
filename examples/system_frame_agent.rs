//! Executable example: register a frame-based agent and stream frames to FEAGI.
//!
//! Usage:
//! 1) Ensure FEAGI is running with a loaded genome.
//! 2) Put PNG/JPG/BMP/TIFF frames in a directory.
//! 3) Run this example with the required environment variables.
//!
//! Example:
//! FEAGI_TEST_FRAME_DIR="/path/to/frames" \
//! FEAGI_TEST_FRAME_LOOPS=3 \
//! FEAGI_TEST_GAZE_X=0.5 \
//! FEAGI_TEST_GAZE_Y=0.5 \
//! FEAGI_TEST_GAZE_MODULATION=0.5 \
//! cargo run --example system_frame_agent
//!
//! Required environment variables:
//! - FEAGI_TEST_FRAME_DIR (path to frame directory)
//!
//! Optional environment variables (defaults shown):
//! - FEAGI_TEST_FRAME_LOOPS (default: 3)
//! - FEAGI_TEST_GAZE_X (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_GAZE_Y (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_GAZE_MODULATION (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_DIFF_THRESHOLD (default: 15; higher drops more unchanged pixels)
//! - FEAGI_TEST_SEGMENTED_CENTER_WIDTH (default: 128)
//! - FEAGI_TEST_SEGMENTED_CENTER_HEIGHT (default: 128)
//! - FEAGI_TEST_SEGMENTED_PERIPHERAL_WIDTH (default: 32)
//! - FEAGI_TEST_SEGMENTED_PERIPHERAL_HEIGHT (default: 32)
//! - FEAGI_TEST_SENSORY_RATE_HZ (optional: requested sensory rate in Hz)
//! - FEAGI_TEST_SENSORY_RATE_STRICT (default: false; true => fail if FEAGI cannot honor rate)
//! - FEAGI_TEST_ALLOW_FEAGI_RATE_UPSHIFT (default: false; true => allow changing FEAGI burst rate)

use anyhow::{Context, Result};
use feagi_agent::clients::async_helpers::tokio_generic_implementations::{
    SensoryRateNegotiationConfig, SensoryRateNegotiationPolicy, TokioDriverConfig,
    TokioEmbodimentAgent,
};
use feagi_agent::clients::SessionTimingConfig;
use feagi_agent::{AgentCapabilities, AgentDescriptor, AuthToken};
use feagi_config::{load_config, FeagiConfig};
use feagi_io::protocol_implementations::zmq::FeagiZmqClientRequesterProperties;
use feagi_sensorimotor::data_pipeline::PipelineStageProperties;
use feagi_sensorimotor::data_types::descriptors::{
    ColorChannelLayout, ColorSpace, ImageXYResolution,
    SegmentedImageFrameProperties, SegmentedXYImageResolutions,
};
use feagi_sensorimotor::data_types::{GazeProperties, ImageFrame, Percentage, Percentage2D};
use feagi_sensorimotor::wrapped_io_data::WrappedIOData;
use feagi_structures::genomic::cortical_area::descriptors::{
    CorticalChannelCount, CorticalChannelIndex, CorticalUnitIndex,
};
use feagi_structures::genomic::cortical_area::io_cortical_area_configuration_flag::FrameChangeHandling;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

/// Example settings sourced from environment variables.
struct ExampleSettings {
    cortical_unit_id: u8,
    color_space: ColorSpace,
    frame_dir: PathBuf,
    frame_loops: usize,
    gaze_x: f32,
    gaze_y: f32,
    gaze_modulation: f32,
    diff_threshold: u8,
    segmented_center_width: u32,
    segmented_center_height: u32,
    segmented_peripheral_width: u32,
    segmented_peripheral_height: u32,
    requested_sensory_rate_hz: Option<f64>,
    sensory_rate_strict: bool,
    allow_feagi_rate_upshift: bool,
}

fn main() -> Result<()> {
    run_example()
}

/// Load the FEAGI configuration using the standard loader.
fn load_feagi_config() -> Result<FeagiConfig> {
    load_config(None, None).context("Failed to load FEAGI configuration")
}

/// Require an environment variable and return its raw string value.
fn require_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("{name} must be set"))
}

/// Parse an environment variable or return a provided default.
fn parse_env_or_default<T>(name: &str, default_value: T) -> Result<T>
where
    T: FromStr + Copy,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(raw) => raw
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("{name} must be a valid value; got '{raw}'; error: {e}")),
        Err(_) => Ok(default_value),
    }
}

/// Parse an optional environment variable.
fn parse_optional_env<T>(name: &str) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(raw) => raw
            .parse::<T>()
            .map(Some)
            .map_err(|e| anyhow::anyhow!("{name} must be a valid value; got '{raw}'; error: {e}")),
        Err(_) => Ok(None),
    }
}

/// Load example settings from environment variables.
fn load_example_settings() -> Result<ExampleSettings> {
    let requested_sensory_rate_hz = parse_optional_env::<f64>("FEAGI_TEST_SENSORY_RATE_HZ")?;
    if let Some(rate_hz) = requested_sensory_rate_hz {
        if !rate_hz.is_finite() || rate_hz <= 0.0 {
            return Err(anyhow::anyhow!(
                "FEAGI_TEST_SENSORY_RATE_HZ must be a finite value > 0, got {}",
                rate_hz
            ));
        }
    }
    Ok(ExampleSettings {
        cortical_unit_id: 0,
        color_space: ColorSpace::Gamma,
        frame_dir: PathBuf::from(require_env("FEAGI_TEST_FRAME_DIR")?),
        frame_loops: parse_env_or_default("FEAGI_TEST_FRAME_LOOPS", 3)?,
        gaze_x: parse_env_or_default("FEAGI_TEST_GAZE_X", 0.5)?,
        gaze_y: parse_env_or_default("FEAGI_TEST_GAZE_Y", 0.5)?,
        gaze_modulation: parse_env_or_default("FEAGI_TEST_GAZE_MODULATION", 0.5)?,
        diff_threshold: parse_env_or_default("FEAGI_TEST_DIFF_THRESHOLD", 15)?,
        segmented_center_width: parse_env_or_default("FEAGI_TEST_SEGMENTED_CENTER_WIDTH", 128)?,
        segmented_center_height: parse_env_or_default("FEAGI_TEST_SEGMENTED_CENTER_HEIGHT", 128)?,
        segmented_peripheral_width: parse_env_or_default(
            "FEAGI_TEST_SEGMENTED_PERIPHERAL_WIDTH",
            32,
        )?,
        segmented_peripheral_height: parse_env_or_default(
            "FEAGI_TEST_SEGMENTED_PERIPHERAL_HEIGHT",
            32,
        )?,
        requested_sensory_rate_hz,
        sensory_rate_strict: parse_env_or_default("FEAGI_TEST_SENSORY_RATE_STRICT", false)?,
        allow_feagi_rate_upshift: parse_env_or_default(
            "FEAGI_TEST_ALLOW_FEAGI_RATE_UPSHIFT",
            false,
        )?,
    })
}

/// Determine if a file extension is supported by the image loader.
fn is_supported_extension(extension: &str) -> bool {
    matches!(extension, "png" | "jpg" | "jpeg" | "bmp" | "tiff")
}

/// Load image frame paths from a directory, sorted by file name.
fn load_frame_paths(frame_dir: &Path) -> Result<Vec<PathBuf>> {
    if !frame_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "FEAGI_TEST_FRAME_DIR must be a directory: {}",
            frame_dir.display()
        ));
    }

    let mut paths: Vec<PathBuf> = fs::read_dir(frame_dir)
        .with_context(|| format!("Failed to read frame directory: {}", frame_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| is_supported_extension(&ext.to_ascii_lowercase()))
                .unwrap_or(false)
        })
        .collect();

    paths.sort();

    if paths.is_empty() {
        return Err(anyhow::anyhow!(
            "No supported frame files found in {}",
            frame_dir.display()
        ));
    }

    Ok(paths)
}

/// Load a single image frame from disk using the requested color space.
fn load_image_frame(path: &Path, color_space: &ColorSpace) -> Result<ImageFrame> {
    let bytes =
        fs::read(path).with_context(|| format!("Failed to read frame: {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| anyhow::anyhow!("Frame missing extension: {}", path.display()))?;

    let frame_result = match extension.as_str() {
        "png" => ImageFrame::new_from_png_bytes(&bytes, color_space),
        "jpg" | "jpeg" => ImageFrame::new_from_jpeg_bytes(&bytes, color_space),
        "bmp" => ImageFrame::new_from_bmp_bytes(&bytes, color_space),
        "tiff" => ImageFrame::new_from_tiff_bytes(&bytes, color_space),
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported frame extension '{}': {}",
                extension,
                path.display()
            ))
        }
    };

    frame_result
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Failed to decode frame bytes")
}

/// Load frames and validate consistent dimensions.
fn load_frame_sequence(frame_paths: &[PathBuf], color_space: &ColorSpace) -> Result<Vec<ImageFrame>> {
    if frame_paths.is_empty() {
        return Err(anyhow::anyhow!("No frame paths provided"));
    }

    let mut frames = Vec::with_capacity(frame_paths.len());
    let mut expected_resolution: Option<ImageXYResolution> = None;
    let mut expected_layout: Option<ColorChannelLayout> = None;

    for path in frame_paths {
        let frame = load_image_frame(path, color_space)?;
        let resolution = frame.get_xy_resolution();
        let layout = *frame.get_channel_layout();

        if let Some(expected) = expected_resolution {
            if resolution.width != expected.width || resolution.height != expected.height {
                return Err(anyhow::anyhow!(
                    "Frame resolution mismatch in {} (expected {}x{}, got {}x{})",
                    path.display(),
                    expected.width,
                    expected.height,
                    resolution.width,
                    resolution.height
                ));
            }
        } else {
            expected_resolution = Some(resolution);
        }

        if let Some(expected) = expected_layout {
            if layout != expected {
                return Err(anyhow::anyhow!(
                    "Frame channel layout mismatch in {} (expected {:?}, got {:?})",
                    path.display(),
                    expected,
                    layout
                ));
            }
        } else {
            expected_layout = Some(layout);
        }

        frames.push(frame);
    }

    Ok(frames)
}

fn format_tcp_endpoint(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("tcp://[{host}]:{port}")
    } else {
        format!("tcp://{host}:{port}")
    }
}

/// Create and connect the embodiment agent using current `feagi-agent` APIs.
fn create_connected_embodiment(
    config: &FeagiConfig,
    settings: &ExampleSettings,
    first_frame: &ImageFrame,
) -> Result<TokioEmbodimentAgent> {
    let agent_descriptor = AgentDescriptor::new("feagi", "system-frame-agent", 1)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Failed to create agent descriptor")?;
    let auth_token = AuthToken::new([0u8; 32]);

    let registration_endpoint =
        format_tcp_endpoint(&config.agent.advertised_host, config.agent.registration_port);
    let registration_properties = Box::new(
        FeagiZmqClientRequesterProperties::new(&registration_endpoint)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to create registration endpoint properties")?,
    );

    let registration_deadline_ms = u64::try_from(
        Duration::from_secs_f64(config.timeouts.service_startup).as_millis(),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("service_startup timeout out of range")?;

    if settings.requested_sensory_rate_hz.is_some() && !settings.allow_feagi_rate_upshift {
        eprintln!(
            "[system_frame_agent] FEAGI_TEST_SENSORY_RATE_HZ set but FEAGI_TEST_ALLOW_FEAGI_RATE_UPSHIFT=false; keeping FEAGI burst rate unchanged."
        );
    }

    let driver = TokioDriverConfig {
        poll_interval: Duration::from_secs_f64(config.neural.burst_engine_timestep),
        timing: SessionTimingConfig {
            heartbeat_interval_ms: config.zmq.client_heartbeat_timeout,
            registration_deadline_ms: Some(registration_deadline_ms),
        },
        sensory_rate_negotiation: if settings.allow_feagi_rate_upshift {
            settings
                .requested_sensory_rate_hz
                .map(|requested_sensory_rate_hz| SensoryRateNegotiationConfig {
                    requested_sensory_rate_hz,
                    feagi_api_host: config.api.advertised_host.clone(),
                    feagi_api_port: config.api.port,
                    api_timeout: Duration::from_secs_f64(config.timeouts.service_startup),
                    policy: if settings.sensory_rate_strict {
                        SensoryRateNegotiationPolicy::Strict
                    } else {
                        SensoryRateNegotiationPolicy::CapAndWarn
                    },
                })
        } else {
            None
        },
    };

    let mut embodiment = TokioEmbodimentAgent::new_unconnected(
        registration_properties,
        agent_descriptor,
        auth_token,
        vec![
            AgentCapabilities::SendSensorData,
            AgentCapabilities::ReceiveMotorData,
        ],
        driver,
    );

    register_vision_device(&mut embodiment, settings, first_frame)?;
    embodiment
        .connect_and_register_spin()
        .context("Failed to connect/register agent")?;
    Ok(embodiment)
}

/// Register segmented vision device in the connector cache.
fn register_vision_device(
    embodiment: &mut TokioEmbodimentAgent,
    settings: &ExampleSettings,
    frame: &ImageFrame,
) -> Result<()> {
    let mut sensor_cache = embodiment.get_embodiment_mut().get_sensor_cache();
    let unit_index = CorticalUnitIndex::from(settings.cortical_unit_id);
    let channel_count = CorticalChannelCount::new(1).context("CorticalChannelCount must be > 0")?;
    let frame_change_handling = FrameChangeHandling::Absolute;
    let image_props = frame.get_image_frame_properties();
    let center_resolution = ImageXYResolution::new(
        settings.segmented_center_width,
        settings.segmented_center_height,
    )
    .context("Invalid segmented center resolution")?;
    let peripheral_resolution = ImageXYResolution::new(
        settings.segmented_peripheral_width,
        settings.segmented_peripheral_height,
    )
    .context("Invalid segmented peripheral resolution")?;
    let segmented_resolutions = SegmentedXYImageResolutions::create_with_same_sized_peripheral(
        center_resolution,
        peripheral_resolution,
    );
    let segmented_props = SegmentedImageFrameProperties::new(
        segmented_resolutions,
        image_props.get_color_channel_layout(),
        image_props.get_color_channel_layout(),
        image_props.get_color_space(),
    );
    let gaze_x = Percentage::new_from_0_1(settings.gaze_x)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Invalid FEAGI_TEST_GAZE_X (expected 0.0-1.0)")?;
    let gaze_y = Percentage::new_from_0_1(settings.gaze_y)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Invalid FEAGI_TEST_GAZE_Y (expected 0.0-1.0)")?;
    let gaze_modulation = Percentage::new_from_0_1(settings.gaze_modulation)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Invalid FEAGI_TEST_GAZE_MODULATION (expected 0.0-1.0)")?;
    let initial_gaze = GazeProperties::new(Percentage2D::new(gaze_x, gaze_y), gaze_modulation);

    sensor_cache
        .segmented_vision_register(
            unit_index,
            channel_count,
            frame_change_handling,
            image_props.clone(),
            segmented_props.clone(),
            initial_gaze.clone(),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Failed to register segmented vision device")?;

    // Align example behavior with desktop controller: quick-diff before segmentation.
    // This avoids near-full-frame sensory injection that can overwhelm FEAGI.
    let quick_diff_stage = PipelineStageProperties::new_image_quick_diff(
        settings.diff_threshold..=u8::MAX,
        Percentage::new_from_0_1(0.0).context("Invalid activity lower bound")?
            ..=Percentage::new_from_0_1(1.0).context("Invalid activity upper bound")?,
        image_props,
    );
    let segmentator_stage = PipelineStageProperties::new_image_frame_segmentator(
        image_props,
        segmented_props,
        initial_gaze,
    );
    sensor_cache
        .segmented_vision_replace_all_stages(
            unit_index,
            CorticalChannelIndex::from(0u32),
            vec![quick_diff_stage, segmentator_stage],
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Failed to install segmented vision quick-diff pipeline")?;

    Ok(())
}

/// Write the next frame into the sensor cache.
fn write_frame(
    embodiment: &mut TokioEmbodimentAgent,
    settings: &ExampleSettings,
    frame: &ImageFrame,
) -> Result<()> {
    let unit_index = CorticalUnitIndex::from(settings.cortical_unit_id);
    let channel_index = CorticalChannelIndex::from(0u32);
    let wrapped = WrappedIOData::ImageFrame(frame.clone());
    let mut sensor_cache = embodiment.get_embodiment_mut().get_sensor_cache();
    sensor_cache
        .segmented_vision_write(unit_index, channel_index, wrapped)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Failed to write segmented vision frame")
}

fn run_example() -> Result<()> {
    let config = load_feagi_config()?;
    let settings = load_example_settings()?;

    let frame_paths = load_frame_paths(&settings.frame_dir)?;
    let frames = load_frame_sequence(&frame_paths, &settings.color_space)?;
    let first_frame = frames
        .first()
        .context("Loaded frames list is empty after validation")?;

    let mut embodiment = create_connected_embodiment(&config, &settings, first_frame)?;
    let frame_interval = Duration::from_secs_f64(config.neural.burst_engine_timestep);

    for _ in 0..settings.frame_loops {
        for frame in &frames {
            write_frame(&mut embodiment, &settings, frame)?;
            embodiment
                .send_stored_sensor_data()
                .context("Failed to encode/send sensory frame")?;
            std::thread::sleep(frame_interval);
        }
    }

    Ok(())
}
