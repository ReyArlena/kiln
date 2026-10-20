//! Safety checks for tests that overwrite a block device.

use std::path::Path;

use anyhow::{Result, bail};

use crate::engine::util::read_trimmed;

/// Refuse a device that is missing, not a block device, mounted anywhere, or
/// the disk the running system lives on. Returns the canonical path.
pub fn check_destructive_target(device: &str) -> Result<String> {
    let path = Path::new(device);
    let canonical =
        std::fs::canonicalize(path).map_err(|_| anyhow::anyhow!("{device} does not exist"))?;
    let canonical_str = canonical.to_string_lossy().to_string();

    let meta = std::fs::metadata(&canonical)?;
    if !is_block_device(&meta) {
        bail!("{device} is not a block device");
    }

    let mounts = read_trimmed("/proc/self/mounts").unwrap_or_default();
    for line in mounts.lines() {
        let Some(source) = line.split_whitespace().next() else {
            continue;
        };
        let Ok(source_canonical) = std::fs::canonicalize(source) else {
            continue;
        };
        let source_str = source_canonical.to_string_lossy().to_string();
        if source_str == canonical_str {
            bail!("{device} is mounted; unmount it first");
        }
        if is_partition_of(&source_str, &canonical_str) {
            bail!("{device} has a mounted partition ({source_str}); unmount it first");
        }
    }

    if let Some(root_disk) = root_disk()
        && (root_disk == canonical_str || is_partition_of(&canonical_str, &root_disk))
    {
        bail!("{device} holds the running system");
    }

    Ok(canonical_str)
}

/// A device that will only be read: must exist and be a block device.
/// Mounted and system disks are fine.
pub fn check_read_target(device: &str) -> Result<String> {
    let canonical =
        std::fs::canonicalize(device).map_err(|_| anyhow::anyhow!("{device} does not exist"))?;
    if !is_block_device(&std::fs::metadata(&canonical)?) {
        bail!("{device} is not a block device");
    }
    Ok(canonical.to_string_lossy().to_string())
}

fn is_block_device(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    meta.file_type().is_block_device()
}

/// `/dev/nvme0n1p2` is a partition of `/dev/nvme0n1`; `/dev/sda1` of `/dev/sda`.
fn is_partition_of(candidate: &str, disk: &str) -> bool {
    match (candidate.strip_prefix("/dev/"), disk.strip_prefix("/dev/")) {
        (Some(c), Some(d)) => crate::engine::devices::is_partition_name(c, d),
        _ => false,
    }
}

use crate::engine::devices::root_disk;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_naming_rules() {
        assert!(is_partition_of("/dev/nvme0n1p2", "/dev/nvme0n1"));
        assert!(is_partition_of("/dev/sda1", "/dev/sda"));
        assert!(!is_partition_of("/dev/sda", "/dev/sda"));
        assert!(!is_partition_of("/dev/sdb1", "/dev/sda"));
        assert!(!is_partition_of("/dev/nvme1n1", "/dev/nvme1"));
    }

    #[test]
    fn nonexistent_device_is_rejected() {
        assert!(check_destructive_target("/dev/kiln-does-not-exist").is_err());
    }
}
