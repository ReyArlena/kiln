//! A snapshot of the machine, stored with every run so results from
//! different days can be compared with confidence.

use std::process::Command;

use serde::{Deserialize, Serialize};

use super::sampler::mem_total_bytes;
use super::util::read_trimmed;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnvSnapshot {
    pub hostname: String,
    pub cpu_model: String,
    pub cpu_count: usize,
    pub board: String,
    pub bios: String,
    pub kernel: String,
    pub cmdline: String,
    pub mem_total_bytes: u64,
    pub governor: String,
    pub iommu: String,
    /// Populated only when running as root (needs `dmidecode`).
    pub dimms: Vec<Dimm>,
    pub nvmes: Vec<NvmeInfo>,
    pub is_root: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dimm {
    pub slot: String,
    pub size: String,
    pub speed: String,
    pub configured_speed: String,
    pub part: String,
    pub serial: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NvmeInfo {
    /// Controller name such as `nvme0`.
    pub name: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
    /// Block device for the first namespace, e.g. `/dev/nvme0n1`.
    pub device: String,
}

impl EnvSnapshot {
    pub fn gather() -> Self {
        let is_root = nix::unistd::Uid::effective().is_root();
        Self {
            hostname: read_trimmed("/proc/sys/kernel/hostname").unwrap_or_default(),
            cpu_model: cpu_model(),
            cpu_count: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            board: format!(
                "{} {}",
                read_trimmed("/sys/class/dmi/id/board_vendor").unwrap_or_default(),
                read_trimmed("/sys/class/dmi/id/board_name").unwrap_or_default()
            )
            .trim()
            .to_string(),
            bios: format!(
                "{} ({})",
                read_trimmed("/sys/class/dmi/id/bios_version").unwrap_or_default(),
                read_trimmed("/sys/class/dmi/id/bios_date").unwrap_or_default()
            ),
            kernel: read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_default(),
            cmdline: read_trimmed("/proc/cmdline").unwrap_or_default(),
            mem_total_bytes: mem_total_bytes(),
            governor: read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
                .unwrap_or_else(|| "unknown".into()),
            iommu: iommu_state(),
            dimms: if is_root {
                dimms_from_dmidecode()
            } else {
                Vec::new()
            },
            nvmes: nvme_controllers(),
            is_root,
        }
    }

    /// Short multi-line summary for headers and reports.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Host    {}", self.hostname),
            format!("CPU     {} ({} threads)", self.cpu_model, self.cpu_count),
            format!("Board   {}", self.board),
            format!("BIOS    {}", self.bios),
            format!("Kernel  {}", self.kernel),
            format!(
                "Memory  {}",
                super::util::format_bytes(self.mem_total_bytes)
            ),
            format!("IOMMU   {}", self.iommu),
        ];
        for d in &self.dimms {
            lines.push(format!(
                "DIMM    {} {} {} {} sn {}",
                d.slot, d.size, d.configured_speed, d.part, d.serial
            ));
        }
        for n in &self.nvmes {
            lines.push(format!(
                "NVMe    {} {} fw {} ({})",
                n.name, n.model, n.firmware, n.device
            ));
        }
        lines
    }
}

fn cpu_model() -> String {
    read_trimmed("/proc/cpuinfo")
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

fn iommu_state() -> String {
    let groups = std::fs::read_dir("/sys/kernel/iommu_groups")
        .map(|d| d.count())
        .unwrap_or(0);
    let cmdline = read_trimmed("/proc/cmdline").unwrap_or_default();
    let mode = cmdline
        .split_whitespace()
        .filter(|w| {
            w.starts_with("iommu=") || w.starts_with("amd_iommu=") || w.starts_with("intel_iommu=")
        })
        .collect::<Vec<_>>()
        .join(" ");
    if groups == 0 {
        "off".to_string()
    } else if mode.is_empty() {
        format!("on ({groups} groups)")
    } else {
        format!("on ({groups} groups, {mode})")
    }
}

/// Parse `dmidecode -t 17` into populated DIMMs. Returns empty on any failure.
fn dimms_from_dmidecode() -> Vec<Dimm> {
    let Ok(output) = Command::new("dmidecode").args(["-t", "17"]).output() else {
        return Vec::new();
    };
    parse_dmidecode_memory(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_dmidecode_memory(text: &str) -> Vec<Dimm> {
    let mut dimms = Vec::new();
    for block in text.split("\n\n") {
        if !block.contains("Memory Device") {
            continue;
        }
        let field = |key: &str| -> String {
            block
                .lines()
                .find_map(|l| {
                    l.trim()
                        .strip_prefix(key)
                        .map(|v| v.trim_start_matches(':').trim().to_string())
                })
                .unwrap_or_default()
        };
        let size = field("Size");
        if size.is_empty() || size.starts_with("No Module") {
            continue;
        }
        dimms.push(Dimm {
            slot: field("Bank Locator"),
            size,
            speed: field("Speed"),
            configured_speed: field("Configured Memory Speed"),
            part: field("Part Number"),
            serial: field("Serial Number"),
        });
    }
    dimms
}

fn nvme_controllers() -> Vec<NvmeInfo> {
    let Ok(entries) = std::fs::read_dir("/sys/class/nvme") else {
        return Vec::new();
    };
    let mut nvmes: Vec<NvmeInfo> = entries
        .flatten()
        .map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let device = std::fs::read_dir(&path)
                .ok()
                .and_then(|d| {
                    let mut ns: Vec<String> = d
                        .flatten()
                        .map(|n| n.file_name().to_string_lossy().to_string())
                        .filter(|n| n.starts_with(&name) && n[name.len()..].starts_with('n'))
                        .collect();
                    ns.sort();
                    ns.into_iter().next()
                })
                .map(|ns| format!("/dev/{ns}"))
                .unwrap_or_default();
            NvmeInfo {
                model: read_trimmed(path.join("model")).unwrap_or_default(),
                firmware: read_trimmed(path.join("firmware_rev")).unwrap_or_default(),
                serial: read_trimmed(path.join("serial")).unwrap_or_default(),
                device,
                name,
            }
        })
        .collect();
    nvmes.sort_by(|a, b| a.name.cmp(&b.name));
    nvmes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmidecode_memory_blocks_parse_and_skip_empty_slots() {
        let text = "\
# dmidecode 3.6
Handle 0x0010, DMI type 17, 92 bytes
Memory Device
\tSize: No Module Installed
\tBank Locator: BANK 0
\tLocator: DIMM 0

Handle 0x0011, DMI type 17, 92 bytes
Memory Device
\tSize: 16 GB
\tLocator: DIMM 1
\tBank Locator: BANK 1
\tSpeed: 6000 MT/s
\tSerial Number: TEST0001
\tPart Number: EXAMPLE-PART-01
\tConfigured Memory Speed: 4800 MT/s
";
        let dimms = parse_dmidecode_memory(text);
        assert_eq!(dimms.len(), 1);
        assert_eq!(dimms[0].slot, "BANK 1");
        assert_eq!(dimms[0].configured_speed, "4800 MT/s");
        assert_eq!(dimms[0].serial, "TEST0001");
    }
}
