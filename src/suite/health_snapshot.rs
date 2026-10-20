//! Collects drive health, PCIe link state and recent kernel errors. Never fails
//! on its own; it exists so the numbers are stored next to the other results.

use anyhow::Result;

use crate::engine::params::{ParamSet, ParamSpec};
use crate::engine::test::{
    BuiltinCtx, Category, Job, JobKind, Outcome, Snapshot, Test, TestInfo, Verdict,
};
use crate::engine::{Dep, EnvSnapshot};

pub struct HealthSnapshot {
    info: TestInfo,
}

impl HealthSnapshot {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "health-snapshot",
                name: "Health snapshot",
                tldr: "Records SMART data, PCIe link speeds and kernel errors. Run it first and last to see what a session changed.",
                description: "\
Captures, for every NVMe controller, `nvme smart-log` (media errors, error \
log entries, temperature, wear) and for every disk `smartctl -a`; the PCIe \
link speed and width of each NVMe from sysfs (a Gen5 drive negotiating \
Gen1 is a slot or signal problem); the IOMMU state; and the tail of the \
kernel log filtered for MCE, AER, NVMe and btrfs messages.

Tools that are missing are simply noted. Nothing is written to any device.

Measures: media_errors and num_err_log_entries per NVMe, when nvme-cli is \
present.

Pass: always, unless a capture could not be started at all. Use the stored \
output and the before/after diff in the report to judge.",
                category: Category::Health,
                uses: &[
                    "nvme smart-log",
                    "smartctl -a",
                    "/sys/bus/pci/devices/*/current_link_{speed,width}",
                    "dmesg",
                ],
                destructive: false,
            },
        }
    }
}

impl Default for HealthSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for HealthSnapshot {
    fn info(&self) -> &TestInfo {
        &self.info
    }

    fn deps(&self) -> Vec<Dep> {
        vec![]
    }

    fn optional_bins(&self) -> Vec<&'static str> {
        vec!["nvme", "smartctl"]
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![]
    }

    fn plan(&self, _params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let mut snapshots = vec![
            Snapshot::new("pcie-links", "sh", &["-c", PCIE_LINKS]),
            Snapshot::new(
                "dmesg-errors",
                "sh",
                &[
                    "-c",
                    "dmesg 2>/dev/null | grep -iE 'mce|machine check|aer|nvme|btrfs.*(csum|corrupt)|i/o error' | tail -40",
                ],
            ),
            Snapshot::new(
                "smartctl-scan",
                "sh",
                &[
                    "-c",
                    "command -v smartctl >/dev/null || { echo 'smartctl not installed'; exit 0; }; for d in $(smartctl --scan | awk '{print $1}'); do echo \"=== $d\"; smartctl -a \"$d\" 2>&1 | grep -iE 'model|serial|firmware|temperature|error|reallocated|pending|wear|percentage used|data units|power on|unsafe'; done",
                ],
            ),
        ];
        for nvme in &env.nvmes {
            snapshots.push(Snapshot::new(&format!("nvme-smart-log-{}", nvme.name), "sh", &["-c", &format!("command -v nvme >/dev/null || {{ echo 'nvme-cli not installed'; exit 0; }}; nvme smart-log /dev/{}", nvme.name)]));
        }
        let nvme_names: Vec<String> = env.nvmes.iter().map(|n| n.name.clone()).collect();
        Ok(Job {
            kind: JobKind::Builtin(Box::new(move |ctx| Box::pin(summarise(ctx, nvme_names)))),
            graphs: vec![],
            expected_secs: Some(5),
            snapshots,
            verdict: Box::new(verdict),
            parser: Box::new(|_| Vec::new()),
        })
    }
}

/// Lists each PCI device's negotiated link, highlighting NVMe controllers.
const PCIE_LINKS: &str = r#"
for dev in /sys/bus/pci/devices/*; do
  class=$(cat "$dev/class" 2>/dev/null)
  case "$class" in 0x010802*|0x0108*) ;; *) continue;; esac
  speed=$(cat "$dev/current_link_speed" 2>/dev/null); width=$(cat "$dev/current_link_width" 2>/dev/null)
  max_speed=$(cat "$dev/max_link_speed" 2>/dev/null); max_width=$(cat "$dev/max_link_width" 2>/dev/null)
  echo "$(basename "$dev") nvme link ${speed:-?} x${width:-?} (max ${max_speed:-?} x${max_width:-?})"
done
echo "iommu groups: $(ls /sys/kernel/iommu_groups 2>/dev/null | wc -l)"
"#;

/// The builtin reads nvme smart-log itself so headline counters land in metrics.
async fn summarise(ctx: BuiltinCtx, nvmes: Vec<String>) -> Result<i32> {
    if which::which("nvme").is_err() {
        ctx.say("nvme-cli not installed, SMART counters not collected (sudo pacman -S nvme-cli)");
        return Ok(0);
    }
    for name in nvmes {
        let out = tokio::process::Command::new("nvme")
            .args(["smart-log", &format!("/dev/{name}")])
            .output()
            .await;
        let Ok(out) = out else { continue };
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |key: &str| -> Option<f64> {
            text.lines()
                .find(|l| l.trim_start().starts_with(key))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| {
                    v.trim()
                        .replace(',', "")
                        .split_whitespace()
                        .next()?
                        .parse()
                        .ok()
                })
        };
        let media = field("media_errors").unwrap_or(-1.0);
        let errlog = field("num_err_log_entries").unwrap_or(-1.0);
        let temp = field("temperature").unwrap_or(-1.0);
        ctx.say(format!(
            "{name}: media_errors={media} num_err_log_entries={errlog} temperature={temp}"
        ));
        ctx.say(format!(
            "KILN_METRIC {name}_media_errors={media} {name}_err_log_entries={errlog}"
        ));
    }
    Ok(0)
}

fn verdict(outcome: &Outcome) -> Verdict {
    match outcome.exit_code {
        Some(0) => Verdict::Pass,
        other => Verdict::Fail(format!("snapshot exited with {other:?}")),
    }
}
