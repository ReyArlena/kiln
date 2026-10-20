//! The contract every test implements, and the job it hands to the runner.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tokio::sync::watch;

use super::deps::Dep;
use super::env::EnvSnapshot;
use super::params::{ParamSet, ParamSpec};

/// Static description shown in the picker, `kiln list` and `kiln info`.
#[derive(Debug, Clone)]
pub struct TestInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// One line: what this test is good for.
    pub tldr: &'static str,
    /// Full breakdown: what runs, what is measured, how pass/fail is decided.
    pub description: &'static str,
    pub category: Category,
    /// External programs or kernel facilities the test relies on.
    pub uses: &'static [&'static str],
    /// True when the test overwrites a block device or otherwise destroys data.
    pub destructive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Cpu,
    Memory,
    Storage,
    Health,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Storage => "Storage",
            Self::Health => "Health",
        }
    }
}

/// A test type. Implementations live in [`crate::suite`].
pub trait Test: Send + Sync {
    fn info(&self) -> &TestInfo;
    fn deps(&self) -> Vec<Dep>;
    fn params(&self) -> Vec<ParamSpec>;
    /// Validate the resolved parameters against the machine and build the job.
    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job>;
}

/// Decides pass/fail once a job has ended.
pub type VerdictFn = Box<dyn Fn(&Outcome) -> Verdict + Send + Sync>;
/// Extracts metrics from one line of output.
pub type ParserFn = Box<dyn Fn(&str) -> Vec<(String, f64)> + Send + Sync>;

/// A series the dashboard should graph while a test runs. The CPU package
/// temperature is always graphed; tests add what else matters to them.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphSpec {
    /// Short gutter label, e.g. `NVME1` or `MB/s`.
    pub label: String,
    pub source: GraphSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphSource {
    /// A hwmon sensor by id, e.g. `nvme1/Composite`.
    Sensor(String),
    /// Average core frequency in GHz.
    CpuFreq,
    /// Overall CPU busy percentage.
    CpuPercent,
    /// Used memory as a percentage of total.
    MemoryPercent,
    /// CPU package watts.
    CpuPower,
    /// GPU watts.
    GpuPower,
    /// The latest value of a parsed metric, sampled each tick.
    Metric(String),
}

impl GraphSpec {
    pub fn sensor(label: &str, id: &str) -> Self {
        Self {
            label: label.into(),
            source: GraphSource::Sensor(id.into()),
        }
    }

    pub fn metric(label: &str, key: &str) -> Self {
        Self {
            label: label.into(),
            source: GraphSource::Metric(key.into()),
        }
    }

    pub fn cpu_freq() -> Self {
        Self {
            label: "GHz".into(),
            source: GraphSource::CpuFreq,
        }
    }

    pub fn cpu_percent() -> Self {
        Self {
            label: "CPU%".into(),
            source: GraphSource::CpuPercent,
        }
    }

    pub fn memory() -> Self {
        Self {
            label: "MEM".into(),
            source: GraphSource::MemoryPercent,
        }
    }

    pub fn cpu_power() -> Self {
        Self {
            label: "CPU W".into(),
            source: GraphSource::CpuPower,
        }
    }

    pub fn gpu_power() -> Self {
        Self {
            label: "GPU W".into(),
            source: GraphSource::GpuPower,
        }
    }
}

/// What the runner actually executes for one test.
pub struct Job {
    pub kind: JobKind,
    /// Extra series for the live dashboard, besides CPU temperature.
    pub graphs: Vec<GraphSpec>,
    /// Rough expected duration, used for progress bars only.
    pub expected_secs: Option<u64>,
    /// Commands captured before and after the main work (SMART, link state, ...).
    pub snapshots: Vec<Snapshot>,
    /// Decide pass/fail from the exit status, parsed metrics and captured output.
    pub verdict: VerdictFn,
    /// Turn each output line into zero or more metrics.
    pub parser: ParserFn,
}

pub enum JobKind {
    /// Spawn an external program. Output is streamed line by line.
    Command { program: String, args: Vec<String> },
    /// Run Rust code in-process. Receives a cancellation signal and an output sink.
    Builtin(BuiltinFn),
}

/// Context handed to builtin jobs.
pub struct BuiltinCtx {
    /// Flips to `true` when a guard trips or the user cancels.
    pub cancel: watch::Receiver<bool>,
    /// Lines written here are logged and parsed exactly like command output.
    pub output: tokio::sync::mpsc::UnboundedSender<String>,
}

impl BuiltinCtx {
    pub fn cancelled(&self) -> bool {
        *self.cancel.borrow()
    }

    pub fn say(&self, line: impl Into<String>) {
        let _ = self.output.send(line.into());
    }
}

/// A builtin returns an exit code so it is judged the same way as a command.
pub type BuiltinFn =
    Box<dyn FnOnce(BuiltinCtx) -> Pin<Box<dyn Future<Output = Result<i32>> + Send>> + Send>;

/// A named command captured before and after the job.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

impl Snapshot {
    pub fn new(name: &str, program: &str, args: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }
}

/// What the verdict function sees once the job ends.
pub struct Outcome<'a> {
    pub exit_code: Option<i32>,
    pub metrics: &'a BTreeMap<String, f64>,
    pub output: &'a [String],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Pass,
    Fail(String),
}

/// Generic parser for `KILN_METRIC key=value` lines emitted by scripts and builtins.
pub fn parse_kiln_metric(line: &str) -> Vec<(String, f64)> {
    let Some(rest) = line.trim().strip_prefix("KILN_METRIC ") else {
        return Vec::new();
    };
    rest.split_whitespace()
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((key.to_string(), value.parse().ok()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kiln_metric_lines_parse() {
        assert_eq!(
            parse_kiln_metric("KILN_METRIC csum_errors=0 diffs=3"),
            vec![("csum_errors".into(), 0.0), ("diffs".into(), 3.0)]
        );
        assert!(parse_kiln_metric("plain log line").is_empty());
        assert!(parse_kiln_metric("KILN_METRIC bad=notanumber").is_empty());
    }
}
