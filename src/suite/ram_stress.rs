//! Memory stress with verification, from the CPU side.

use anyhow::Result;

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{Category, Job, JobKind, Test, TestInfo};
use crate::engine::{Dep, EnvSnapshot};

use super::stress_ng;

pub struct RamStress {
    info: TestInfo,
}

impl RamStress {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "ram-stress",
                name: "RAM stress (verified)",
                tldr: "Fills most of RAM with patterns and reads them back. Catches bad DIMMs, bad slots and unstable memory clocks.",
                description: "\
Runs stress-ng's vm stressor across several workers, each mapping a slice \
of memory, writing a pattern, and reading it back to verify. Cycles through \
every vm method (walking ones, galloping patterns, random, ...). This is a \
CPU-side test: it proves the CPU can write and read RAM correctly, but does \
not exercise device DMA into RAM. Pair it with a storage test for that.

Measures: bogo ops, bogo ops per second, max temperature.

Pass: no `fail:` lines and exit 0. Fail: any pattern mismatch. Abort: guard cap.",
                category: Category::Memory,
                uses: &[
                    "stress-ng --vm --vm-method all --verify",
                    "hwmon temperature sensors",
                ],
                destructive: false,
            },
        }
    }
}

impl Default for RamStress {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for RamStress {
    fn info(&self) -> &TestInfo {
        &self.info
    }

    fn deps(&self) -> Vec<Dep> {
        vec![Dep::Bin("stress-ng")]
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec {
                key: "duration",
                label: "Duration",
                help: "How long to run",
                kind: ParamKind::Duration {
                    default: 600,
                    min: 5,
                    max: 24 * 3600,
                },
            },
            ParamSpec {
                key: "workers",
                label: "Workers",
                help: "vm stressor count. 0 uses half the threads",
                kind: ParamKind::Int {
                    default: 0,
                    min: 0,
                    max: 1024,
                    zero_means_auto: true,
                },
            },
            ParamSpec {
                key: "percent",
                label: "Memory %",
                help: "Share of free memory to use across all workers",
                kind: ParamKind::Int {
                    default: 80,
                    min: 5,
                    max: 95,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let duration = params["duration"].as_u64().unwrap_or(600);
        let workers = match params["workers"].as_i64().unwrap_or(0) {
            0 => (env.cpu_count / 2).max(1),
            n => n as usize,
        };
        let percent = params["percent"].as_i64().unwrap_or(80);
        // stress-ng takes a percentage of free memory per total, so the
        // figure is spread across workers for us.
        let bytes_arg = format!("{percent}%");
        Ok(Job {
            kind: JobKind::Command {
                program: "stress-ng".into(),
                args: vec![
                    "--vm".into(),
                    workers.to_string(),
                    "--vm-bytes".into(),
                    bytes_arg,
                    "--vm-method".into(),
                    "all".into(),
                    "--verify".into(),
                    "--timeout".into(),
                    format!("{duration}s"),
                    "--metrics-brief".into(),
                    "--log-brief".into(),
                ],
            },
            graphs: vec![],
            expected_secs: Some(duration),
            snapshots: vec![],
            verdict: Box::new(stress_ng::verdict),
            parser: Box::new(stress_ng::parse_metrics_line),
        })
    }
}
