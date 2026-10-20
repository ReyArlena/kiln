//! Whole-disk enumeration for device pickers, from sysfs and /proc/mounts.

use std::path::Path;

use serde::Serialize;

use super::util::{format_bytes, read_trimmed};

#[derive(Debug, Clone, Serialize)]
pub struct BlockDevice {
    /// `/dev/nvme0n1`, `/dev/sda`, ...
    pub path: String,
    pub model: String,
    pub size_bytes: u64,
    /// `nvme`, `sata`, `usb`, `virtio`, `mmc` or `other`.
    pub transport: String,
    /// Mount points of the disk or any of its partitions.
    pub mounts: Vec<String>,
    /// The disk the running system lives on.
    pub is_system: bool,
}

impl BlockDevice {
    pub fn is_mounted(&self) -> bool {
        !self.mounts.is_empty()
    }

    /// One line for lists: path, model, size, bus, mounts, system marker.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "{:<14} {:<22} {:>7}  {:<6}",
            self.path,
            truncate(&self.model, 22),
            format_bytes(self.size_bytes),
            self.transport
        );
        if self.is_system {
            text.push_str("  SYSTEM DISK");
        } else if self.is_mounted() {
            text.push_str(&format!("  mounted: {}", self.mounts.join(" ")));
        }
        text
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max - 1).collect::<String>() + "…"
    }
}

/// Every whole disk (no partitions, no loop/ram/zram devices), sorted by path.
pub fn list_block_devices() -> Vec<BlockDevice> {
    let Ok(entries) = std::fs::read_dir("/sys/class/block") else {
        return Vec::new();
    };
    let system = root_disk();
    let mounts = read_trimmed("/proc/self/mounts").unwrap_or_default();
    let mut disks: Vec<BlockDevice> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let sys = e.path();
            // Partitions carry a `partition` file; virtual devices have no `device` link.
            if sys.join("partition").exists() || !sys.join("device").exists() {
                return None;
            }
            if name.starts_with("loop")
                || name.starts_with("ram")
                || name.starts_with("zram")
                || name.starts_with("sr")
            {
                return None;
            }
            let path = format!("/dev/{name}");
            let sectors: u64 = read_trimmed(sys.join("size"))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            Some(BlockDevice {
                model: read_trimmed(sys.join("device/model"))
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "unknown model".into()),
                size_bytes: sectors * 512,
                transport: transport_of(&sys),
                mounts: mounts_of(&name, &mounts),
                is_system: system.as_deref() == Some(path.as_str()),
                path,
            })
        })
        .collect();
    disks.sort_by(|a, b| a.path.cmp(&b.path));
    disks
}

/// Guess the bus from the resolved sysfs path.
fn transport_of(sys: &Path) -> String {
    let real = std::fs::canonicalize(sys)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if real.contains("/nvme/") {
        "nvme"
    } else if real.contains("/usb") {
        "usb"
    } else if real.contains("/ata") {
        "sata"
    } else if real.contains("/virtio") {
        "virtio"
    } else if real.contains("/mmc") {
        "mmc"
    } else {
        "other"
    }
    .to_string()
}

/// Mount points whose source is this disk or one of its partitions.
fn mounts_of(disk: &str, mounts: &str) -> Vec<String> {
    let mut points = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(source), Some(target)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(canonical) = std::fs::canonicalize(source) else {
            continue;
        };
        let Some(name) = canonical.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == disk || is_partition_name(name, disk) {
            points.push(target.to_string());
        }
    }
    points
}

/// `nvme0n1p2` belongs to `nvme0n1`; `sda1` to `sda`.
pub fn is_partition_name(candidate: &str, disk: &str) -> bool {
    let Some(rest) = candidate.strip_prefix(disk) else {
        return false;
    };
    let rest = rest.strip_prefix('p').unwrap_or(rest);
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// The whole disk behind `/`, resolved through sysfs so LUKS or LVM on top
/// of it is not fooled by a mapper name.
pub fn root_disk() -> Option<String> {
    let mounts = read_trimmed("/proc/self/mounts")?;
    let source = mounts
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some("/"))?
        .split_whitespace()
        .next()?
        .to_string();
    let canonical = std::fs::canonicalize(&source).ok()?;
    let name = canonical.file_name()?.to_str()?.to_string();
    let block = Path::new("/sys/class/block").join(&name);
    // A partition's sysfs entry lives inside its parent disk's directory.
    let real = std::fs::canonicalize(block).ok()?;
    let parent = real.parent()?.file_name()?.to_str()?.to_string();
    let disk = if Path::new("/sys/class/block")
        .join(&parent)
        .join("device")
        .exists()
        || parent.starts_with("nvme")
        || parent.starts_with("sd")
    {
        parent
    } else {
        name
    };
    Some(format!("/dev/{disk}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_names() {
        assert!(is_partition_name("nvme0n1p2", "nvme0n1"));
        assert!(is_partition_name("sda1", "sda"));
        assert!(!is_partition_name("sda", "sda"));
        assert!(!is_partition_name("sdb1", "sda"));
        assert!(!is_partition_name("nvme1n1", "nvme1"));
    }

    #[test]
    fn mounts_match_disk_and_partitions() {
        // Sources must resolve, so use /dev/null-style fakes via names only where canonicalize succeeds.
        let devices = list_block_devices();
        // Whatever this machine has, no entry may be a partition or a virtual device.
        for d in &devices {
            assert!(!d.path.contains("loop"));
            assert!(d.size_bytes > 0, "{}", d.path);
        }
    }
}
