//! Read-only drive benchmark with fio: sequential MB/s and 4k random IOPS.

use anyhow::Result;

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{Category, Job, JobKind, Outcome, Snapshot, Test, TestInfo, Verdict};
use crate::engine::{Dep, EnvSnapshot};

use super::device::check_read_target;

pub struct DiskRead {
    info: TestInfo,
}

impl DiskRead {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "disk-read",
                name: "Disk Read Benchmark",
                tldr: "Sequential MB/s and random 4k IOPS, read-only. Safe on the system drive. A Gen5 drive reading like Gen1 is a slot or link problem.",
                description: "\
Two fio jobs straight from the block device with direct I/O, both \
read-only: sequential 1 MB reads at queue depth 16 for throughput, then \
4 KB random reads at queue depth 32 for IOPS. Each runs for a fixed time. \
Nothing is written, so this is safe on a mounted or system disk, and it \
needs no spare device.

Read it together with the PCIe link line from the health snapshot: an NVMe \
drive far below its rated speed with a link negotiated below its maximum \
points at the slot, the cable or the CPU I/O die rather than the drive.

Needs root to open the raw device.

Pass: fio exits 0.",
                category: Category::Benchmark,
                uses: &[
                    "fio --readonly --rw=read --bs=1M",
                    "fio --readonly --rw=randread --bs=4k",
                ],
                destructive: false,
            },
        }
    }
}

impl Default for DiskRead {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for DiskRead {
    fn info(&self) -> &TestInfo {
        &self.info
    }

    fn deps(&self) -> Vec<Dep> {
        vec![Dep::Bin("fio"), Dep::Root]
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec {
                key: "device",
                label: "Device",
                help: "Block device to read, e.g. /dev/nvme0n1. Not modified",
                kind: ParamKind::ReadDevice,
            },
            ParamSpec {
                key: "seconds",
                label: "Seconds per job",
                help: "Runtime of each of the two jobs",
                kind: ParamKind::Int {
                    default: 10,
                    min: 3,
                    max: 600,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, _env: &EnvSnapshot) -> Result<Job> {
        let device = check_read_target(params["device"].as_str().unwrap_or(""))?;
        let seconds = params["seconds"].as_i64().unwrap_or(10);
        let controller = device
            .trim_start_matches("/dev/")
            .split('n')
            .next()
            .unwrap_or("");
        let snapshots = if device.contains("nvme") {
            vec![Snapshot::new(
                "pcie-link",
                "sh",
                &[
                    "-c",
                    &format!(
                        "for d in /sys/class/nvme/{controller}/device; do echo \"$(cat $d/current_link_speed) x$(cat $d/current_link_width) (max $(cat $d/max_link_speed) x$(cat $d/max_link_width))\"; done 2>/dev/null"
                    ),
                ],
            )]
        } else {
            vec![]
        };
        let common = |name: &str, rw: &str, bs: &str, depth: u32| -> Vec<String> {
            vec![
                format!("--name={name}"),
                format!("--rw={rw}"),
                format!("--bs={bs}"),
                format!("--iodepth={depth}"),
                format!("--runtime={seconds}"),
                "--time_based".into(),
            ]
        };
        let mut args = vec![
            format!("--filename={device}"),
            "--readonly".into(),
            "--direct=1".into(),
            "--ioengine=libaio".into(),
            "--output-format=terse".into(),
            "--terse-version=3".into(),
            "--eta=never".into(),
        ];
        args.extend(common("seq", "read", "1M", 16));
        let mut rand = common("rand", "randread", "4k", 32);
        rand.insert(1, "--stonewall".into());
        args.extend(rand);
        Ok(Job {
            kind: JobKind::Command {
                program: "fio".into(),
                args,
            },
            graphs: vec![],
            expected_secs: Some(seconds as u64 * 2 + 2),
            snapshots,
            verdict: Box::new(verdict),
            parser: Box::new(parse_terse),
        })
    }
}

/// One terse v3 line per job: field 2 is the job name, 6 the read bandwidth
/// in KiB/s, 7 the read IOPS.
fn parse_terse(line: &str) -> Vec<(String, f64)> {
    let f: Vec<&str> = line.split(';').collect();
    if f.len() < 50 || f[0] != "3" {
        return Vec::new();
    }
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
    match f[2] {
        "seq" => num(6)
            .map(|kib| vec![("seq_read_mb_s".to_string(), kib / 1024.0)])
            .unwrap_or_default(),
        "rand" => num(7)
            .map(|iops| vec![("rand_read_iops".to_string(), iops)])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn verdict(outcome: &Outcome) -> Verdict {
    match outcome.exit_code {
        Some(0) if outcome.metrics.contains_key("seq_read_mb_s") => Verdict::Pass,
        Some(0) => Verdict::Fail("fio produced no results".into()),
        Some(code) => Verdict::Fail(format!("fio exited with code {code}")),
        None => Verdict::Fail("fio was killed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_lines_map_to_the_right_job() {
        let mut f = vec!["0"; 130];
        f[0] = "3";
        f[2] = "seq";
        f[6] = "3145728";
        assert_eq!(
            parse_terse(&f.join(";")),
            vec![("seq_read_mb_s".to_string(), 3072.0)]
        );
        f[2] = "rand";
        f[7] = "650000";
        assert_eq!(
            parse_terse(&f.join(";")),
            vec![("rand_read_iops".to_string(), 650000.0)]
        );
        assert!(parse_terse("fio-3.36").is_empty());
    }
}
