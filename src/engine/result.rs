//! What a run produces: one JSON file per run plus a log per test.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use super::env::EnvSnapshot;
use super::guards::Trip;
use super::params::ParamSet;
use super::plan::Plan;
use super::sampler::Sample;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Pass,
    Fail(String),
    Aborted(Trip),
    /// Could not start: missing dependency or invalid parameters.
    Skipped(String),
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail(_) => "FAIL",
            Self::Aborted(_) => "ABORT",
            Self::Skipped(_) => "SKIP",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Pass => String::new(),
            Self::Fail(why) | Self::Skipped(why) => why.clone(),
            Self::Aborted(trip) => trip.describe(),
        }
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestResult {
    pub index: usize,
    pub test_id: String,
    pub name: String,
    pub params: ParamSet,
    pub started: DateTime<Local>,
    pub ended: DateTime<Local>,
    pub status: Status,
    pub exit_code: Option<i32>,
    pub metrics: BTreeMap<String, f64>,
    pub max_temp_c: Option<f32>,
    /// Relative to the run directory.
    pub log_file: String,
    /// Captured snapshots as `(name, before, after)`.
    pub snapshots: Vec<SnapshotCapture>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotCapture {
    pub name: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub kiln_version: String,
    pub started: DateTime<Local>,
    pub ended: DateTime<Local>,
    /// True while the run is still going. A file left with this set means
    /// kiln stopped unexpectedly (crash, power loss, killed).
    #[serde(default)]
    pub in_progress: bool,
    pub plan: Plan,
    pub env: EnvSnapshot,
    pub results: Vec<TestResult>,
    pub samples: Vec<Sample>,
}

impl Run {
    pub fn all_passed(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.status.is_pass())
    }

    /// Exit code for the headless CLI: 0 pass, 1 fail, 3 aborted or skipped.
    pub fn exit_code(&self) -> i32 {
        if self.all_passed() {
            0
        } else if self
            .results
            .iter()
            .any(|r| matches!(r.status, Status::Fail(_)))
        {
            1
        } else {
            3
        }
    }

    pub fn save(&self, dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("run.json");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Every saved run under `runs_dir`, newest first. Unreadable entries are skipped.
    pub fn list(runs_dir: &Path) -> Vec<(PathBuf, Run)> {
        let Ok(entries) = std::fs::read_dir(runs_dir) else {
            return Vec::new();
        };
        let mut runs: Vec<(PathBuf, Run)> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|dir| Run::load(&dir.join("run.json")).ok().map(|run| (dir, run)))
            .collect();
        runs.sort_by(|a, b| b.1.started.cmp(&a.1.started));
        runs
    }

    /// Remove the oldest runs so at most `keep` remain. Zero keeps everything.
    pub fn prune(runs_dir: &Path, keep: usize) {
        if keep == 0 {
            return;
        }
        for (dir, _) in Run::list(runs_dir).into_iter().skip(keep) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Run id derived from the start time, safe for directory names.
    pub fn make_id(started: DateTime<Local>) -> String {
        started.format("%Y-%m-%dT%H-%M-%S").to_string()
    }
}
