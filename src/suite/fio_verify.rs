//! Direct I/O write with checksum verification on a whole block device.

use anyhow::Result;

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{Category, Job, JobKind, Outcome, Snapshot, Test, TestInfo, Verdict};
use crate::engine::util::format_bytes;
use crate::engine::{Dep, EnvSnapshot};

use super::device::check_destructive_target;

pub struct FioVerify {
    info: TestInfo,
}

impl FioVerify {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "fio-verify",
                name: "fio write + verify",
                tldr: "Writes a drive with checksummed blocks then reads them back. Catches data corrupted on the way to or from the drive.",
                description: "\
Uses fio to write the chosen device with direct I/O (bypassing the page \
cache) in large sequential blocks, embedding a CRC32C checksum in every \
block, then reads the whole region back and verifies each checksum. Because \
the data crosses the PCIe link and the IOMMU twice via DMA, this exercises \
the CPU I/O die, the slot, the drive controller and the NAND together.

DESTRUCTIVE: everything on the device is overwritten. The device must be \
unmounted and must not be the disk the running system lives on. kiln \
checks both before starting.

Measures: write and read bandwidth, verification errors, max temperature.

Pass: fio exits 0 with zero errors. Fail: any verify mismatch or I/O error. \
Abort: guard cap.",
                category: Category::Storage,
                uses: &[
                    "fio --rw=write --verify=crc32c --direct=1",
                    "nvme smart-log (before/after, when available)",
                ],
                destructive: true,
            },
        }
    }
}

impl Default for FioVerify {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for FioVerify {
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
                help: "Block device to overwrite, e.g. /dev/nvme1n1",
                kind: ParamKind::BlockDevice,
            },
            ParamSpec {
                key: "size",
                label: "Size",
                help: "How much of the device to write, e.g. 20G",
                kind: ParamKind::Bytes { default: 20 << 30 },
            },
            ParamSpec {
                key: "block_size",
                label: "Block size",
                help: "I/O size per request",
                kind: ParamKind::Choice {
                    choices: &["128k", "1M", "4M"],
                    default: "1M",
                },
            },
            ParamSpec {
                key: "iodepth",
                label: "Queue depth",
                help: "Outstanding requests",
                kind: ParamKind::Int {
                    default: 8,
                    min: 1,
                    max: 128,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, _env: &EnvSnapshot) -> Result<Job> {
        let device = check_destructive_target(params["device"].as_str().unwrap_or(""))?;
        let size = params["size"].as_u64().unwrap_or(20 << 30);
        let block_size = params["block_size"].as_str().unwrap_or("1M").to_string();
        let iodepth = params["iodepth"].as_i64().unwrap_or(8);
        let nvme_name = device
            .trim_start_matches("/dev/")
            .split('n')
            .next()
            .map(|c| format!("/dev/{c}"));
        let snapshots = match nvme_name {
            Some(ctrl) if device.contains("nvme") => vec![Snapshot::new(
                "nvme-smart-log",
                "nvme",
                &["smart-log", &ctrl],
            )],
            _ => vec![],
        };
        Ok(Job {
            kind: JobKind::Command {
                program: "fio".into(),
                args: vec![
                    "--name=kiln-verify".into(),
                    format!("--filename={device}"),
                    "--rw=write".into(),
                    format!("--bs={block_size}"),
                    format!("--size={}", format_bytes(size)),
                    "--direct=1".into(),
                    "--ioengine=libaio".into(),
                    format!("--iodepth={iodepth}"),
                    "--verify=crc32c".into(),
                    "--do_verify=1".into(),
                    "--verify_fatal=1".into(),
                    "--output-format=terse".into(),
                    "--terse-version=3".into(),
                    "--eta=never".into(),
                ],
            },
            graphs: vec![],
            expected_secs: None,
            snapshots,
            verdict: Box::new(verdict),
            parser: Box::new(parse_terse_line),
        })
    }
}

/// fio terse v3 output is one `;`-separated line. Field 7 is write bandwidth
/// in KiB/s, field 48 is read bandwidth in KiB/s (verify phase), field 5 is
/// the error count. Indices are zero-based after splitting.
fn parse_terse_line(line: &str) -> Vec<(String, f64)> {
    let fields: Vec<&str> = line.split(';').collect();
    if fields.len() < 100 || fields[0] != "3" {
        return Vec::new();
    }
    let num = |i: usize| fields.get(i).and_then(|v| v.parse::<f64>().ok());
    let mut out = Vec::new();
    if let Some(err) = num(4) {
        out.push(("fio_errors".into(), err));
    }
    if let Some(read_kib) = num(6) {
        out.push(("read_mb_per_sec".into(), read_kib / 1024.0));
    }
    if let Some(write_kib) = num(47) {
        out.push(("write_mb_per_sec".into(), write_kib / 1024.0));
    }
    out
}

fn verdict(outcome: &Outcome) -> Verdict {
    if let Some(bad) = outcome
        .output
        .iter()
        .find(|l| l.contains("verify: bad") || l.contains("verify failed"))
    {
        return Verdict::Fail(bad.trim().to_string());
    }
    if outcome.metrics.get("fio_errors").is_some_and(|e| *e > 0.0) {
        return Verdict::Fail(format!(
            "fio reported {} error(s)",
            outcome.metrics["fio_errors"]
        ));
    }
    match outcome.exit_code {
        Some(0) => Verdict::Pass,
        Some(code) => Verdict::Fail(format!("fio exited with code {code}")),
        None => Verdict::Fail("fio was killed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_line_extracts_errors_and_bandwidth() {
        let mut fields = vec!["0"; 130];
        fields[0] = "3";
        fields[4] = "0";
        fields[6] = "2048000";
        fields[47] = "1024000";
        let line = fields.join(";");
        let m = parse_terse_line(&line);
        assert!(m.contains(&("fio_errors".to_string(), 0.0)));
        assert!(m.contains(&("read_mb_per_sec".to_string(), 2000.0)));
        assert!(m.contains(&("write_mb_per_sec".to_string(), 1000.0)));
        assert!(parse_terse_line("fio-3.36").is_empty());
    }
}
