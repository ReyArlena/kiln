//! CPU stress with result verification, bounded by the temperature guard.

use anyhow::Result;

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{Category, Job, JobKind, Snapshot, Test, TestInfo};
use crate::engine::{Dep, EnvSnapshot};

use super::stress_ng;

pub struct CpuStress {
    info: TestInfo,
}

impl CpuStress {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "cpu-stress",
                name: "CPU stress (verified)",
                tldr: "Heats the CPU with checked arithmetic. Catches cores that compute wrong under load or heat.",
                description: "\
Runs stress-ng with one CPU stressor per thread, cycling through every \
CPU method (integer, float, matrix, crc, hashing, ...) and verifying each \
result against a known-good answer. Watches the package temperature every \
sample and sends SIGINT to stress-ng if the cap is reached, so the test \
stops rather than the machine throttling or shutting down.

Measures: bogo operations and bogo ops per second (a rough throughput \
figure, comparable between runs on the same machine), and the maximum \
temperature seen.

Pass: stress-ng exits 0 with no `fail:` lines. Fail: any verification \
mismatch or non-zero exit. Abort: temperature cap or time cap hit.",
                category: Category::Cpu,
                uses: &[
                    "stress-ng --cpu --cpu-method all --verify",
                    "hwmon temperature sensors",
                    "cpufreq",
                ],
                destructive: false,
            },
        }
    }
}

impl Default for CpuStress {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for CpuStress {
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
                help: "How long to run, e.g. 10m or 1h",
                kind: ParamKind::Duration {
                    default: 600,
                    min: 5,
                    max: 24 * 3600,
                },
            },
            ParamSpec {
                key: "workers",
                label: "Workers",
                help: "Stressor count. 0 uses every thread",
                kind: ParamKind::Int {
                    default: 0,
                    min: 0,
                    max: 1024,
                    zero_means_auto: true,
                },
            },
            ParamSpec {
                key: "method",
                label: "Method",
                help: "Which CPU method to cycle. `all` rotates through every one",
                kind: ParamKind::Choice {
                    choices: &[
                        "all",
                        "matrixprod",
                        "int64",
                        "double",
                        "crc16",
                        "fft",
                        "prime",
                    ],
                    default: "all",
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let duration = params["duration"].as_u64().unwrap_or(600);
        let workers = match params["workers"].as_i64().unwrap_or(0) {
            0 => env.cpu_count,
            n => n as usize,
        };
        let method = params["method"].as_str().unwrap_or("all").to_string();
        Ok(Job {
            kind: JobKind::Command {
                program: "stress-ng".into(),
                args: vec![
                    "--cpu".into(),
                    workers.to_string(),
                    "--cpu-method".into(),
                    method,
                    "--verify".into(),
                    "--timeout".into(),
                    format!("{duration}s"),
                    "--metrics-brief".into(),
                    "--log-brief".into(),
                ],
            },
            graphs: vec![],
            expected_secs: Some(duration),
            snapshots: vec![Snapshot::new(
                "dmesg-errors",
                "sh",
                &[
                    "-c",
                    "dmesg 2>/dev/null | grep -iE 'mce|machine check|hardware error' | tail -20",
                ],
            )],
            verdict: Box::new(stress_ng::verdict),
            parser: Box::new(stress_ng::parse_metrics_line),
        })
    }
}
