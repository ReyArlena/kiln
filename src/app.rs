//! Shared startup: paths, settings, logging, the test registry and the
//! machine snapshot. Both the CLI and the TUI begin here.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use crate::config::{AppConfig, Paths, SensorChoice};
use crate::engine::runner::RunContext;
use crate::engine::sampler::Sampler;
use crate::engine::{EnvSnapshot, Registry};

pub struct AppState {
    pub paths: Paths,
    pub config: AppConfig,
    pub registry: Registry,
    pub env: EnvSnapshot,
}

impl AppState {
    pub fn load() -> Result<Self> {
        let paths = Paths::resolve()?;
        paths.ensure_dirs()?;
        let config = AppConfig::load(&paths.config_file)?;
        init_logging(&paths);
        tracing::info!(version = env!("CARGO_PKG_VERSION"), "kiln starting");
        Ok(Self {
            paths,
            config,
            registry: Registry::builtin(),
            env: EnvSnapshot::gather(),
        })
    }

    /// An app state rooted in a scratch directory, with no logging. For tests.
    pub fn sandboxed(base: &Path) -> Result<Self> {
        let paths = Paths::under(base);
        paths.ensure_dirs()?;
        let config = AppConfig::load(&paths.config_file)?;
        Ok(Self {
            paths,
            config,
            registry: Registry::builtin(),
            env: EnvSnapshot::gather(),
        })
    }

    pub fn save_config(&self) -> Result<()> {
        self.config.save(&self.paths.config_file)?;
        self.paths.restore_owner(&self.paths.config_file);
        Ok(())
    }

    /// Delete the config file and go back to defaults.
    pub fn reset_config(&mut self) -> Result<()> {
        if self.paths.config_file.exists() {
            std::fs::remove_file(&self.paths.config_file)?;
        }
        self.config = AppConfig::default();
        Ok(())
    }

    pub fn sampler(&self) -> Sampler {
        let preferred = match &self.config.sensor {
            SensorChoice::Auto => None,
            SensorChoice::Named(id) => Some(id.as_str()),
        };
        Sampler::new(preferred)
    }

    /// Build the context for a new run. The run directory is derived from
    /// the current time so each run is separate.
    pub fn run_context(&self) -> RunContext {
        let run_id = crate::engine::Run::make_id(chrono::Local::now());
        let paths = self.paths.clone();
        RunContext {
            registry: self.registry.clone(),
            env: self.env.clone(),
            sampler: self.sampler(),
            sample_interval: Duration::from_millis(self.config.sample_interval_ms.max(200)),
            run_dir: self.paths.runs_dir.join(run_id),
            profiles_dir: self.paths.profiles_dir.clone(),
            on_file_created: Arc::new(move |p: &Path| paths.restore_owner(p)),
        }
    }

    pub fn is_root(&self) -> bool {
        self.env.is_root
    }

    /// The command to relaunch as root. Uses the full binary path because
    /// `sudo` resets `PATH` and would not find a per-user install.
    pub fn sudo_hint(&self) -> String {
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "kiln".into());
        format!("sudo {exe}")
    }
}

/// Log to `kiln.log` in the data directory. `KILN_LOG=debug` raises the level.
fn init_logging(paths: &Paths) {
    let Some(parent) = paths.log_file.parent() else {
        return;
    };
    let Some(name) = paths.log_file.file_name() else {
        return;
    };
    let appender = tracing_appender::rolling::never(parent, name);
    let filter = EnvFilter::try_from_env("KILN_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(appender)
        .with_ansi(false)
        .try_init();
    paths.restore_owner(&paths.log_file);
}
