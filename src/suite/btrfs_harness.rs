//! The "installer harness": format a device with btrfs, copy a large tree of
//! real files onto it through the page cache, then scrub and compare.

use anyhow::Result;

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{Category, Job, JobKind, Outcome, Snapshot, Test, TestInfo, Verdict};
use crate::engine::{Dep, EnvSnapshot};

use super::device::check_destructive_target;

pub struct BtrfsHarness {
    info: TestInfo,
}

impl BtrfsHarness {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "btrfs-harness",
                name: "btrfs install harness",
                tldr: "Mimics an OS install: format, copy thousands of files, scrub, compare. The fastest way to reproduce install-time corruption.",
                description: "\
Formats the device with btrfs (checksummed, zstd compressed), mounts it, \
copies the source directory onto it with `cp -a` (buffered writes through \
the page cache, exactly like a distro installer), syncs, drops caches, then \
runs `btrfs scrub` the requested number of times. Scrub re-reads every \
block and checks its checksum, so any data that was corrupted on the way \
to the drive, or that corrupts on the way back, is counted. Finally a \
sample of files is compared byte for byte against the source.

Scrub counts that change between passes point at the read path (data on \
the drive is fine but comes back wrong); stable counts point at the write \
path. Either way the drive, slot, CPU I/O die and RAM are all in the loop, \
which is why this is paired with single-component swaps when hunting a fault.

DESTRUCTIVE: the device is wiped. kiln refuses mounted devices and the \
system disk.

Measures: copy size and time, scrub errors per pass, differing files.

Pass: zero scrub errors on every pass, zero differing files, script reached \
the end. Fail: anything else. Abort: guard cap.",
                category: Category::Storage,
                uses: &[
                    "wipefs",
                    "mkfs.btrfs",
                    "mount -o compress=zstd:1,ssd",
                    "cp -a",
                    "sync + drop_caches",
                    "btrfs scrub start -B",
                    "btrfs device stats",
                    "cmp",
                ],
                destructive: true,
            },
        }
    }
}

impl Default for BtrfsHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for BtrfsHarness {
    fn info(&self) -> &TestInfo {
        &self.info
    }

    fn deps(&self) -> Vec<Dep> {
        vec![
            Dep::Bin("mkfs.btrfs"),
            Dep::Bin("btrfs"),
            Dep::Bin("wipefs"),
            Dep::Bin("cmp"),
            Dep::Root,
        ]
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec {
                key: "device",
                label: "Device",
                help: "Block device to wipe, e.g. /dev/nvme1n1",
                kind: ParamKind::BlockDevice,
            },
            ParamSpec {
                key: "source",
                label: "Source dir",
                help: "Tree to copy. /usr gives a few GB of real files",
                kind: ParamKind::Path { default: "/usr" },
            },
            ParamSpec {
                key: "scrubs",
                label: "Scrub passes",
                help: "Scrub repeats; varying counts expose read-path faults",
                kind: ParamKind::Int {
                    default: 3,
                    min: 1,
                    max: 20,
                    zero_means_auto: false,
                },
            },
            ParamSpec {
                key: "compare_files",
                label: "Files to compare",
                help: "How many copied files to cmp against the source",
                kind: ParamKind::Int {
                    default: 2000,
                    min: 0,
                    max: 1_000_000,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, _env: &EnvSnapshot) -> Result<Job> {
        let device = check_destructive_target(params["device"].as_str().unwrap_or(""))?;
        let source = params["source"].as_str().unwrap_or("/usr").to_string();
        let scrubs = params["scrubs"].as_i64().unwrap_or(3);
        let compare = params["compare_files"].as_i64().unwrap_or(2000);
        Ok(Job {
            kind: JobKind::Command {
                program: "bash".into(),
                args: vec![
                    "-c".into(),
                    SCRIPT.into(),
                    "kiln-harness".into(),
                    device,
                    source,
                    scrubs.to_string(),
                    compare.to_string(),
                ],
            },
            expected_secs: None,
            snapshots: vec![Snapshot::new(
                "dmesg-btrfs",
                "sh",
                &[
                    "-c",
                    "dmesg 2>/dev/null | grep -iE 'btrfs|nvme|aer|mce' | tail -30",
                ],
            )],
            verdict: Box::new(verdict),
            parser: Box::new(|_| Vec::new()),
        })
    }
}

/// Runs as `bash -c SCRIPT kiln-harness DEVICE SOURCE SCRUBS COMPARE`.
/// Every result is reported as `KILN_METRIC` lines so the engine parses them.
const SCRIPT: &str = r#"
set -u
D=$1; SRC=$2; SCRUBS=$3; COMPARE=$4
M=$(mktemp -d /tmp/kiln-harness.XXXXXX)
cleanup() { umount "$M" 2>/dev/null; rmdir "$M" 2>/dev/null; }
trap cleanup EXIT
trap 'echo "harness interrupted"; exit 130' INT TERM

echo "harness start device=$D source=$SRC scrubs=$SCRUBS"
wipefs -a "$D" >/dev/null || { echo "WIPEFS_FAIL"; exit 1; }
mkfs.btrfs -f -q -L kiln-harness "$D" || { echo "MKFS_FAIL"; exit 1; }
mount -o compress=zstd:1,ssd "$D" "$M" || { echo "MOUNT_FAIL"; exit 1; }

t0=$(date +%s)
echo "copy start"
cp -a "$SRC" "$M/tree" 2>"$M/.cp_err"; cp_rc=$?
err_lines=$(wc -l < "$M/.cp_err"); rm -f "$M/.cp_err"
sync
t1=$(date +%s)
size_kb=$(du -sk "$M/tree" | cut -f1)
echo "copy done rc=$cp_rc stderr_lines=$err_lines size_kb=$size_kb secs=$((t1-t0))"
echo "KILN_METRIC copy_secs=$((t1-t0)) copy_mb=$((size_kb/1024)) cp_stderr_lines=$err_lines"
echo 3 > /proc/sys/vm/drop_caches

total_errors=0
for i in $(seq 1 "$SCRUBS"); do
  out=$(btrfs scrub start -B "$M" 2>&1)
  errs=$(echo "$out" | grep -E "Error summary" | grep -oE "[0-9]+" | paste -sd+ - | bc 2>/dev/null || echo 0)
  [ -z "$errs" ] && errs=0
  echo "$out" | grep -E "Error summary|Corrected|Uncorrectable|Total to scrub" | sed "s/^/scrub $i: /"
  echo "KILN_METRIC scrub${i}_errors=$errs"
  total_errors=$((total_errors+errs))
done
echo "KILN_METRIC scrub_errors_total=$total_errors"
btrfs device stats "$M" | sed 's/^/stats: /'
corruption=$(btrfs device stats "$M" | grep corruption_errs | grep -oE "[0-9]+$" || echo 0)
echo "KILN_METRIC corruption_errs=${corruption:-0}"

bad=0; checked=0
if [ "$COMPARE" -gt 0 ]; then
  echo "compare start (up to $COMPARE files)"
  while IFS= read -r f; do
    checked=$((checked+1))
    cmp -s "$SRC/$f" "$M/tree/$f" || { bad=$((bad+1)); [ "$bad" -le 20 ] && echo "DIFF $f"; }
  done < <(cd "$SRC" && find . -type f -readable 2>/dev/null | head -n "$COMPARE")
fi
echo "compare done checked=$checked differing=$bad"
echo "KILN_METRIC compared=$checked diffs=$bad"
umount "$M"
echo "harness FINISHED"
"#;

fn verdict(outcome: &Outcome) -> Verdict {
    let m = outcome.metrics;
    let finished = outcome
        .output
        .iter()
        .any(|l| l.contains("harness FINISHED"));
    if let Some(stage) = outcome.output.iter().find(|l| l.ends_with("_FAIL")) {
        return Verdict::Fail(stage.trim().to_string());
    }
    let scrub = m.get("scrub_errors_total").copied().unwrap_or(f64::NAN);
    let diffs = m.get("diffs").copied().unwrap_or(f64::NAN);
    let corruption = m.get("corruption_errs").copied().unwrap_or(0.0);
    if scrub > 0.0 || corruption > 0.0 {
        return Verdict::Fail(format!(
            "{scrub} scrub error(s), {corruption} device corruption error(s)"
        ));
    }
    if diffs > 0.0 {
        return Verdict::Fail(format!("{diffs} file(s) differ from source"));
    }
    if !finished || scrub.is_nan() || diffs.is_nan() {
        return Verdict::Fail(format!(
            "harness did not finish (exit {:?})",
            outcome.exit_code
        ));
    }
    Verdict::Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn outcome<'a>(metrics: &'a BTreeMap<String, f64>, output: &'a [String]) -> Outcome<'a> {
        Outcome {
            exit_code: Some(0),
            metrics,
            output,
        }
    }

    #[test]
    fn verdict_needs_zero_errors_and_a_finished_marker() {
        let finished = vec!["harness FINISHED".to_string()];
        let mut m = BTreeMap::new();
        m.insert("scrub_errors_total".into(), 0.0);
        m.insert("diffs".into(), 0.0);
        assert_eq!(verdict(&outcome(&m, &finished)), Verdict::Pass);

        m.insert("scrub_errors_total".into(), 3.0);
        assert!(matches!(verdict(&outcome(&m, &finished)), Verdict::Fail(_)));

        m.insert("scrub_errors_total".into(), 0.0);
        assert!(matches!(verdict(&outcome(&m, &[])), Verdict::Fail(_)));
    }
}
