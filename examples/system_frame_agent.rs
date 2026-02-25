//! Executable example: register a frame-based agent and stream frames to FEAGI.
//!
//! Demo folder structure: each demo has an `assets/` subfolder (images, videos) and
//! an optional `genome.json`. If genome.json is present, it is loaded on FEAGI via
//! the REST API before starting sensory streaming.
//!
//! Usage:
//! 1) Ensure FEAGI is running (genome optional if using --demo-dir with genome.json).
//! 2) Either set --demo-dir to a demo folder (e.g. examples/demo1) or provide --frame-dir.
//! 3) With --demo-dir, frame_dir defaults to <demo-dir>/assets and genome to <demo-dir>/genome.json.
//!
//! Example:
//! cargo run --example system_frame_agent -- --demo-dir ./examples/demo1
//!
//! Or with explicit paths:
//! cargo run --example system_frame_agent -- \
//!   --settings-toml ./examples/demo1/settings.toml \
//!   --frame-dir ./examples/demo1/assets
//!
//! Precedence (lowest to highest):
//! 1) Built-in defaults
//! 2) `--settings-toml` values (or root keys if no section is present)
//! 3) CLI flags
//! 4) `FEAGI_TEST_*` environment variables
//!
//! Environment variable overrides (defaults shown):
//! - FEAGI_TEST_DEMO_DIR (demo folder; frame_dir defaults to <demo-dir>/assets)
//! - FEAGI_TEST_FRAME_DIR (required if no demo_dir, or overrides demo assets path)
//! - FEAGI_TEST_FRAME_LOOPS (default: 3)
//! - FEAGI_TEST_GAZE_X (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_GAZE_Y (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_GAZE_MODULATION (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_BRIGHTNESS (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_CONTRAST (default: 0.5, range: 0.0-1.0)
//! - FEAGI_TEST_DIFF_THRESHOLD (default: 15; higher drops more unchanged pixels)
//! - FEAGI_TEST_SEGMENTED_CENTER_WIDTH (default: 128)
//! - FEAGI_TEST_SEGMENTED_CENTER_HEIGHT (default: 128)
//! - FEAGI_TEST_SEGMENTED_PERIPHERAL_WIDTH (default: 32)
//! - FEAGI_TEST_SEGMENTED_PERIPHERAL_HEIGHT (default: 32)
//! - FEAGI_TEST_SENSORY_RATE_HZ (optional: requested sensory rate in Hz)
//! - FEAGI_TEST_SENSORY_RATE_STRICT (default: false; true => fail if FEAGI cannot honor rate)
//! - FEAGI_TEST_ALLOW_FEAGI_RATE_UPSHIFT (default: false; true => allow changing FEAGI burst rate)

use anyhow::{Context, Result};
use clap::Parser;
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
    ColorChannelLayout, ColorSpace, ImageXYResolution, SegmentedImageFrameProperties,
    SegmentedXYImageResolutions,
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

/// Final runtime settings after applying all config sources.
struct ExampleSettings {
    cortical_unit_id: u8,
    color_space: ColorSpace,
    frame_dir: PathBuf,
    /// If set, load this genome on FEAGI via API before starting sensory streaming.
    genome_path: Option<PathBuf>,
    frame_loops: usize,
    gaze_x: f32,
    gaze_y: f32,
    gaze_modulation: f32,
    brightness: f32,
    contrast: f32,
    diff_threshold: u8,
    segmented_center_width: u32,
    segmented_center_height: u32,
    segmented_peripheral_width: u32,
    segmented_peripheral_height: u32,
    requested_sensory_rate_hz: Option<f64>,
    sensory_rate_strict: bool,
    allow_feagi_rate_upshift: bool,
}

/// CLI overrides for this example.
#[derive(Debug, Parser)]
#[command(name = "system_frame_agent")]
#[command(about = "Stream image frames to FEAGI using segmented vision")]
struct CliArgs {
    /// Optional TOML settings file path. Reads [system_frame_agent] if present, else root keys.
    #[arg(long)]
    settings_toml: Option<PathBuf>,
    /// Demo folder (standard structure: assets/, genome.json). Sets frame_dir to <demo-dir>/assets if not overridden.
    #[arg(long)]
    demo_dir: Option<PathBuf>,
    #[arg(long)]
    frame_dir: Option<PathBuf>,
    #[arg(long)]
    frame_loops: Option<usize>,
    #[arg(long)]
    gaze_x: Option<f32>,
    #[arg(long)]
    gaze_y: Option<f32>,
    #[arg(long)]
    gaze_modulation: Option<f32>,
    #[arg(long)]
    brightness: Option<f32>,
    #[arg(long)]
    contrast: Option<f32>,
    #[arg(long)]
    diff_threshold: Option<u8>,
    #[arg(long)]
    segmented_center_width: Option<u32>,
    #[arg(long)]
    segmented_center_height: Option<u32>,
    #[arg(long)]
    segmented_peripheral_width: Option<u32>,
    #[arg(long)]
    segmented_peripheral_height: Option<u32>,
    #[arg(long)]
    requested_sensory_rate_hz: Option<f64>,
    #[arg(long)]
    sensory_rate_strict: Option<bool>,
    #[arg(long)]
    allow_feagi_rate_upshift: Option<bool>,
}

/// Partial overrides loaded from TOML/CLI/env.
#[derive(Debug, Default, Clone)]
struct ExampleSettingsOverrides {
    demo_dir: Option<PathBuf>,
    frame_dir: Option<PathBuf>,
    frame_loops: Option<usize>,
    gaze_x: Option<f32>,
    gaze_y: Option<f32>,
    gaze_modulation: Option<f32>,
    brightness: Option<f32>,
    contrast: Option<f32>,
    diff_threshold: Option<u8>,
    segmented_center_width: Option<u32>,
    segmented_center_height: Option<u32>,
    segmented_peripheral_width: Option<u32>,
    segmented_peripheral_height: Option<u32>,
    requested_sensory_rate_hz: Option<f64>,
    sensory_rate_strict: Option<bool>,
    allow_feagi_rate_upshift: Option<bool>,
}

/// Mutable settings accumulator with built-in defaults.
#[derive(Debug)]
struct ExampleSettingsDraft {
    demo_dir: Option<PathBuf>,
    frame_dir: Option<PathBuf>,
    frame_loops: usize,
    gaze_x: f32,
    gaze_y: f32,
    gaze_modulation: f32,
    brightness: f32,
    contrast: f32,
    diff_threshold: u8,
    segmented_center_width: u32,
    segmented_center_height: u32,
    segmented_peripheral_width: u32,
    segmented_peripheral_height: u32,
    requested_sensory_rate_hz: Option<f64>,
    sensory_rate_strict: bool,
    allow_feagi_rate_upshift: bool,
}

impl Default for ExampleSettingsDraft {
    fn default() -> Self {
        Self {
            demo_dir: None,
            frame_dir: None,
            frame_loops: 3,
            gaze_x: 0.5,
            gaze_y: 0.5,
            gaze_modulation: 1.0,
            brightness: 0.5,
            contrast: 0.5,
            diff_threshold: 15,
            segmented_center_width: 128,
            segmented_center_height: 128,
            segmented_peripheral_width: 32,
            segmented_peripheral_height: 32,
            requested_sensory_rate_hz: None,
            sensory_rate_strict: false,
            allow_feagi_rate_upshift: false,
        }
    }
}

impl ExampleSettingsDraft {
    /// Apply sparse overrides from one source in precedence order.
    fn apply_overrides(&mut self, overrides: ExampleSettingsOverrides) {
        if let Some(value) = overrides.demo_dir {
            self.demo_dir = Some(value);
        }
        if let Some(value) = overrides.frame_dir {
            self.frame_dir = Some(value);
        }
        if let Some(value) = overrides.frame_loops {
            self.frame_loops = value;
        }
        if let Some(value) = overrides.gaze_x {
            self.gaze_x = value;
        }
        if let Some(value) = overrides.gaze_y {
            self.gaze_y = value;
        }
        if let Some(value) = overrides.gaze_modulation {
            self.gaze_modulation = value;
        }
        if let Some(value) = overrides.brightness {
            self.brightness = value;
        }
        if let Some(value) = overrides.contrast {
            self.contrast = value;
        }
        if let Some(value) = overrides.diff_threshold {
            self.diff_threshold = value;
        }
        if let Some(value) = overrides.segmented_center_width {
            self.segmented_center_width = value;
        }
        if let Some(value) = overrides.segmented_center_height {
            self.segmented_center_height = value;
        }
        if let Some(value) = overrides.segmented_peripheral_width {
            self.segmented_peripheral_width = value;
        }
        if let Some(value) = overrides.segmented_peripheral_height {
            self.segmented_peripheral_height = value;
        }
        if let Some(value) = overrides.requested_sensory_rate_hz {
            self.requested_sensory_rate_hz = Some(value);
        }
        if let Some(value) = overrides.sensory_rate_strict {
            self.sensory_rate_strict = value;
        }
        if let Some(value) = overrides.allow_feagi_rate_upshift {
            self.allow_feagi_rate_upshift = value;
        }
    }
}

fn main() -> Result<()> {
    let cli_args = CliArgs::parse();
    run_example(&cli_args)
}

/// Load the FEAGI configuration using the standard loader.
fn load_feagi_config() -> Result<FeagiConfig> {
    load_config(None, None).context("Failed to load FEAGI configuration")
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

fn toml_optional_f32(table: &toml::value::Table, key: &str) -> Result<Option<f32>> {
    match table.get(key) {
        Some(value) => {
            let number = value
                .as_float()
                .or_else(|| value.as_integer().map(|val| val as f64))
                .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be a number"))?;
            Ok(Some(number as f32))
        }
        None => Ok(None),
    }
}

fn toml_optional_f64(table: &toml::value::Table, key: &str) -> Result<Option<f64>> {
    match table.get(key) {
        Some(value) => {
            let number = value
                .as_float()
                .or_else(|| value.as_integer().map(|val| val as f64))
                .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be a number"))?;
            Ok(Some(number))
        }
        None => Ok(None),
    }
}

fn toml_optional_u8(table: &toml::value::Table, key: &str) -> Result<Option<u8>> {
    match table.get(key) {
        Some(value) => {
            let integer = value
                .as_integer()
                .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be an integer"))?;
            let parsed = u8::try_from(integer)
                .map_err(|_| anyhow::anyhow!("Key '{key}' out of range for u8: {integer}"))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn toml_optional_u32(table: &toml::value::Table, key: &str) -> Result<Option<u32>> {
    match table.get(key) {
        Some(value) => {
            let integer = value
                .as_integer()
                .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be an integer"))?;
            let parsed = u32::try_from(integer)
                .map_err(|_| anyhow::anyhow!("Key '{key}' out of range for u32: {integer}"))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn toml_optional_usize(table: &toml::value::Table, key: &str) -> Result<Option<usize>> {
    match table.get(key) {
        Some(value) => {
            let integer = value
                .as_integer()
                .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be an integer"))?;
            let parsed = usize::try_from(integer)
                .map_err(|_| anyhow::anyhow!("Key '{key}' out of range for usize: {integer}"))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn toml_optional_bool(table: &toml::value::Table, key: &str) -> Result<Option<bool>> {
    match table.get(key) {
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be a boolean")),
        None => Ok(None),
    }
}

fn toml_optional_path(table: &toml::value::Table, key: &str) -> Result<Option<PathBuf>> {
    match table.get(key) {
        Some(value) => value
            .as_str()
            .map(|s| Some(PathBuf::from(s)))
            .ok_or_else(|| anyhow::anyhow!("Key '{key}' must be a string path")),
        None => Ok(None),
    }
}

/// Load optional settings overrides from TOML.
fn load_toml_overrides(path: &Path) -> Result<ExampleSettingsOverrides> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read settings TOML: {}", path.display()))?;
    let parsed: toml::Value = raw
        .parse()
        .with_context(|| format!("Failed to parse TOML: {}", path.display()))?;
    let root = parsed
        .as_table()
        .ok_or_else(|| anyhow::anyhow!("Settings TOML root must be a table"))?;
    let table = if let Some(section) = root.get("system_frame_agent") {
        section
            .as_table()
            .ok_or_else(|| anyhow::anyhow!("[system_frame_agent] must be a TOML table"))?
    } else {
        root
    };

    Ok(ExampleSettingsOverrides {
        demo_dir: toml_optional_path(table, "demo_dir")?,
        frame_dir: toml_optional_path(table, "frame_dir")?,
        frame_loops: toml_optional_usize(table, "frame_loops")?,
        gaze_x: toml_optional_f32(table, "gaze_x")?,
        gaze_y: toml_optional_f32(table, "gaze_y")?,
        gaze_modulation: toml_optional_f32(table, "gaze_modulation")?,
        brightness: toml_optional_f32(table, "brightness")?,
        contrast: toml_optional_f32(table, "contrast")?,
        diff_threshold: toml_optional_u8(table, "diff_threshold")?,
        segmented_center_width: toml_optional_u32(table, "segmented_center_width")?,
        segmented_center_height: toml_optional_u32(table, "segmented_center_height")?,
        segmented_peripheral_width: toml_optional_u32(table, "segmented_peripheral_width")?,
        segmented_peripheral_height: toml_optional_u32(table, "segmented_peripheral_height")?,
        requested_sensory_rate_hz: toml_optional_f64(table, "requested_sensory_rate_hz")?,
        sensory_rate_strict: toml_optional_bool(table, "sensory_rate_strict")?,
        allow_feagi_rate_upshift: toml_optional_bool(table, "allow_feagi_rate_upshift")?,
    })
}

/// Build sparse overrides from CLI arguments.
fn cli_overrides(cli_args: &CliArgs) -> ExampleSettingsOverrides {
    ExampleSettingsOverrides {
        demo_dir: cli_args.demo_dir.clone(),
        frame_dir: cli_args.frame_dir.clone(),
        frame_loops: cli_args.frame_loops,
        gaze_x: cli_args.gaze_x,
        gaze_y: cli_args.gaze_y,
        gaze_modulation: cli_args.gaze_modulation,
        brightness: cli_args.brightness,
        contrast: cli_args.contrast,
        diff_threshold: cli_args.diff_threshold,
        segmented_center_width: cli_args.segmented_center_width,
        segmented_center_height: cli_args.segmented_center_height,
        segmented_peripheral_width: cli_args.segmented_peripheral_width,
        segmented_peripheral_height: cli_args.segmented_peripheral_height,
        requested_sensory_rate_hz: cli_args.requested_sensory_rate_hz,
        sensory_rate_strict: cli_args.sensory_rate_strict,
        allow_feagi_rate_upshift: cli_args.allow_feagi_rate_upshift,
    }
}

/// Build sparse overrides from FEAGI_TEST_* env variables.
fn env_overrides() -> Result<ExampleSettingsOverrides> {
    Ok(ExampleSettingsOverrides {
        demo_dir: parse_optional_env::<String>("FEAGI_TEST_DEMO_DIR")?.map(PathBuf::from),
        frame_dir: parse_optional_env::<String>("FEAGI_TEST_FRAME_DIR")?.map(PathBuf::from),
        frame_loops: parse_optional_env::<usize>("FEAGI_TEST_FRAME_LOOPS")?,
        gaze_x: parse_optional_env::<f32>("FEAGI_TEST_GAZE_X")?,
        gaze_y: parse_optional_env::<f32>("FEAGI_TEST_GAZE_Y")?,
        gaze_modulation: parse_optional_env::<f32>("FEAGI_TEST_GAZE_MODULATION")?,
        brightness: parse_optional_env::<f32>("FEAGI_TEST_BRIGHTNESS")?,
        contrast: parse_optional_env::<f32>("FEAGI_TEST_CONTRAST")?,
        diff_threshold: parse_optional_env::<u8>("FEAGI_TEST_DIFF_THRESHOLD")?,
        segmented_center_width: parse_optional_env::<u32>("FEAGI_TEST_SEGMENTED_CENTER_WIDTH")?,
        segmented_center_height: parse_optional_env::<u32>("FEAGI_TEST_SEGMENTED_CENTER_HEIGHT")?,
        segmented_peripheral_width: parse_optional_env::<u32>(
            "FEAGI_TEST_SEGMENTED_PERIPHERAL_WIDTH",
        )?,
        segmented_peripheral_height: parse_optional_env::<u32>(
            "FEAGI_TEST_SEGMENTED_PERIPHERAL_HEIGHT",
        )?,
        requested_sensory_rate_hz: parse_optional_env::<f64>("FEAGI_TEST_SENSORY_RATE_HZ")?,
        sensory_rate_strict: parse_optional_env::<bool>("FEAGI_TEST_SENSORY_RATE_STRICT")?,
        allow_feagi_rate_upshift: parse_optional_env::<bool>(
            "FEAGI_TEST_ALLOW_FEAGI_RATE_UPSHIFT",
        )?,
    })
}

/// Load example settings from TOML, CLI, and environment variables.
fn load_example_settings(cli_args: &CliArgs) -> Result<ExampleSettings> {
    let mut draft = ExampleSettingsDraft::default();

    if let Some(settings_toml_path) = &cli_args.settings_toml {
        draft.apply_overrides(load_toml_overrides(settings_toml_path)?);
    }
    draft.apply_overrides(cli_overrides(cli_args));
    draft.apply_overrides(env_overrides()?);

    if let Some(rate_hz) = draft.requested_sensory_rate_hz {
        if !rate_hz.is_finite() || rate_hz <= 0.0 {
            return Err(anyhow::anyhow!(
                "Requested sensory rate must be a finite value > 0, got {}",
                rate_hz
            ));
        }
    }
    if !(0.0..=1.0).contains(&draft.brightness) {
        return Err(anyhow::anyhow!(
            "Brightness must be in [0.0, 1.0], got {}",
            draft.brightness
        ));
    }
    if !(0.0..=1.0).contains(&draft.contrast) {
        return Err(anyhow::anyhow!(
            "Contrast must be in [0.0, 1.0], got {}",
            draft.contrast
        ));
    }

    // Resolve frame_dir and genome_path. If frame_dir points to a demo folder (has assets/
    // subdir), use that folder as demo root: frames from <path>/assets, genome from <path>/genome.json.
    let (frame_dir, genome_path) = match (&draft.frame_dir, &draft.demo_dir) {
        (Some(fd), _) if fd.join("assets").is_dir() => {
            let assets = fd.join("assets");
            let gp = fd.join("genome.json");
            (assets, if gp.is_file() { Some(gp) } else { None })
        }
        (Some(fd), _) => (
            fd.clone(),
            draft.demo_dir.and_then(|d| {
                let p = d.join("genome.json");
                if p.is_file() {
                    Some(p)
                } else {
                    None
                }
            }),
        ),
        (None, Some(d)) => (d.join("assets"), {
            let p = d.join("genome.json");
            if p.is_file() {
                Some(p)
            } else {
                None
            }
        }),
        (None, None) => {
            return Err(anyhow::anyhow!(
                "frame_dir is required (use --demo-dir, --frame-dir, FEAGI_TEST_DEMO_DIR, FEAGI_TEST_FRAME_DIR, or settings TOML)"
            ));
        }
    };

    Ok(ExampleSettings {
        cortical_unit_id: 0,
        color_space: ColorSpace::Gamma,
        frame_dir,
        genome_path,
        frame_loops: draft.frame_loops,
        gaze_x: draft.gaze_x,
        gaze_y: draft.gaze_y,
        gaze_modulation: draft.gaze_modulation,
        brightness: draft.brightness,
        contrast: draft.contrast,
        diff_threshold: draft.diff_threshold,
        segmented_center_width: draft.segmented_center_width,
        segmented_center_height: draft.segmented_center_height,
        segmented_peripheral_width: draft.segmented_peripheral_width,
        segmented_peripheral_height: draft.segmented_peripheral_height,
        requested_sensory_rate_hz: draft.requested_sensory_rate_hz,
        sensory_rate_strict: draft.sensory_rate_strict,
        allow_feagi_rate_upshift: draft.allow_feagi_rate_upshift,
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
fn load_frame_sequence(
    frame_paths: &[PathBuf],
    color_space: &ColorSpace,
) -> Result<Vec<ImageFrame>> {
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

/// Load a genome file into FEAGI via POST /v1/genome/upload. Blocks until FEAGI sends an explicit
/// HTTP response (success or error). No client-side timeout: we wait for FEAGI's reply whether it
/// arrives in milliseconds or minutes. Only on HTTP success do we return and start streaming.
fn load_genome_via_feagi_api(config: &FeagiConfig, genome_path: &Path) -> Result<()> {
    let json_str = fs::read_to_string(genome_path)
        .with_context(|| format!("Failed to read genome file: {}", genome_path.display()))?;
    let genome_json: serde_json::Value = serde_json::from_str(&json_str)
        .with_context(|| format!("Invalid JSON in genome file: {}", genome_path.display()))?;

    let host = &config.api.advertised_host;
    let port = config.api.port;
    let url = if host.contains(':') {
        format!("http://[{}]:{}/v1/genome/upload", host, port)
    } else {
        format!("http://{}:{}/v1/genome/upload", host, port)
    };

    let client = reqwest::blocking::Client::builder()
        .build()
        .context("Failed to create HTTP client")?;

    eprintln!(
        "[system_frame_agent] Loading genome from {} into FEAGI at {} (waiting for FEAGI response) ...",
        genome_path.display(),
        url
    );
    let response = client
        .post(&url)
        .json(&genome_json)
        .send()
        .with_context(|| format!("Failed to POST genome to {}", url))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(anyhow::anyhow!(
            "FEAGI genome upload failed: {} {}",
            status,
            body
        ));
    }
    eprintln!("[system_frame_agent] FEAGI returned success; starting sensory streaming.");
    Ok(())
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

    let registration_endpoint = format_tcp_endpoint(
        &config.agent.advertised_host,
        config.agent.registration_port,
    );
    let registration_properties = Box::new(
        FeagiZmqClientRequesterProperties::new(&registration_endpoint)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to create registration endpoint properties")?,
    );

    let registration_deadline_ms =
        u64::try_from(Duration::from_secs_f64(config.timeouts.service_startup).as_millis())
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

/// Map normalized brightness [0.0, 1.0] to processor offset [-255, 255].
fn map_brightness_to_offset(brightness: f32) -> i32 {
    ((brightness * 2.0 - 1.0) * 255.0).round() as i32
}

/// Write the next frame into the sensor cache.
fn write_frame(
    embodiment: &mut TokioEmbodimentAgent,
    settings: &ExampleSettings,
    frame: &ImageFrame,
) -> Result<()> {
    let unit_index = CorticalUnitIndex::from(settings.cortical_unit_id);
    let channel_index = CorticalChannelIndex::from(0u32);
    let mut adjusted_frame = frame.clone();
    adjusted_frame.change_brightness(map_brightness_to_offset(settings.brightness));
    adjusted_frame.change_contrast(settings.contrast);
    let wrapped = WrappedIOData::ImageFrame(adjusted_frame);
    let mut sensor_cache = embodiment.get_embodiment_mut().get_sensor_cache();
    sensor_cache
        .segmented_vision_write(unit_index, channel_index, wrapped)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("Failed to write segmented vision frame")
}

fn run_example(cli_args: &CliArgs) -> Result<()> {
    let config = load_feagi_config()?;
    let settings = load_example_settings(cli_args)?;

    if let Some(ref genome_path) = settings.genome_path {
        load_genome_via_feagi_api(&config, genome_path)?;
    }

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
