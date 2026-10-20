//! User-editable settings, persisted as TOML in the config directory.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::engine::Guards;

/// Which temperature sensor the guard watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorChoice {
    /// Pick the CPU package sensor automatically (k10temp Tctl, coretemp Package, ...).
    Auto,
    /// A specific `<hwmon name>/<label>` pair, for example `k10temp/Tctl`.
    Named(String),
}

impl Default for SensorChoice {
    fn default() -> Self {
        Self::Auto
    }
}

/// The always-visible sensor bar in the TUI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarConfig {
    pub enabled: bool,
    /// CPU package temperature (the guard sensor).
    pub cpu_temp: bool,
    /// Overall CPU busy percentage.
    pub cpu_usage: bool,
    /// Average core frequency.
    pub cpu_freq: bool,
    /// Used memory as a percentage of total.
    pub memory: bool,
    /// Composite temperature of each NVMe drive.
    pub nvme_temp: bool,
    /// GPU edge temperature when an amdgpu/nouveau sensor exists.
    pub gpu_temp: bool,
    /// Any other sensor ids (`chip/label`) to show, e.g. `k10temp/Tccd1`.
    pub extra_sensors: Vec<String>,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_temp: true,
            cpu_usage: true,
            cpu_freq: false,
            memory: true,
            nvme_temp: true,
            gpu_temp: false,
            extra_sensors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Default guards applied to every new run. Each run can override them.
    pub guards: Guards,
    /// Sensor used for the temperature guard and the live graph.
    pub sensor: SensorChoice,
    /// How often sensors are read, in milliseconds.
    pub sample_interval_ms: u64,
    /// Enable mouse support in the TUI.
    pub mouse: bool,
    /// Ask for typed confirmation before destructive tests in the TUI.
    pub confirm_destructive: bool,
    /// How many past runs to keep on disk. Zero keeps everything.
    pub runs_to_keep: usize,
    pub status_bar: StatusBarConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            guards: Guards::default(),
            sensor: SensorChoice::Auto,
            sample_interval_ms: 1000,
            mouse: true,
            confirm_destructive: true,
            runs_to_keep: 50,
            status_bar: StatusBarConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load from disk, falling back to defaults if the file does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_toml() {
        let mut config = AppConfig::default();
        config.guards.temp_max_c = Some(85.0);
        config.sensor = SensorChoice::Named("k10temp/Tctl".into());

        let text = toml::to_string(&config).unwrap();
        let back: AppConfig = toml::from_str(&text).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let back: AppConfig = toml::from_str("mouse = false\n").unwrap();
        assert!(!back.mouse);
        assert_eq!(
            back.sample_interval_ms,
            AppConfig::default().sample_interval_ms
        );
    }
}
