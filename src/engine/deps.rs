//! Dependency declarations and the startup check that resolves them.

use std::path::Path;

use nix::unistd::Uid;
use serde::Serialize;

/// Something a test needs before it can run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Dep {
    /// An executable on `PATH`.
    Bin(&'static str),
    /// Root privileges (raw device access, dmidecode, dropping caches).
    Root,
    /// A hwmon chip, e.g. `k10temp`.
    Hwmon(&'static str),
    /// A sysfs path that must exist.
    Sysfs(&'static str),
}

impl Dep {
    pub fn label(&self) -> String {
        match self {
            Self::Bin(name) => name.to_string(),
            Self::Root => "root".to_string(),
            Self::Hwmon(name) => format!("hwmon:{name}"),
            Self::Sysfs(path) => path.to_string(),
        }
    }

    /// Check this dependency on the current machine.
    pub fn check(&self) -> DepStatus {
        let ok = match self {
            Self::Bin(name) => which::which(name).is_ok(),
            Self::Root => Uid::effective().is_root(),
            Self::Hwmon(name) => hwmon_present(name),
            Self::Sysfs(path) => Path::new(path).exists(),
        };
        DepStatus {
            dep: self.clone(),
            ok,
            hint: if ok { None } else { Some(self.hint()) },
        }
    }

    /// What the user should do about a missing dependency.
    fn hint(&self) -> String {
        match self {
            Self::Bin(name) => install_hint(name),
            Self::Root => "relaunch with sudo".to_string(),
            Self::Hwmon(name) => format!("kernel module {name} not loaded or unsupported CPU"),
            Self::Sysfs(path) => format!("{path} missing; kernel feature not available"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DepStatus {
    #[serde(serialize_with = "serialize_dep")]
    pub dep: Dep,
    pub ok: bool,
    pub hint: Option<String>,
}

fn serialize_dep<S: serde::Serializer>(dep: &Dep, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&dep.label())
}

/// Result of checking every dependency a test declares.
#[derive(Debug, Clone, Serialize)]
pub struct DepReport {
    pub statuses: Vec<DepStatus>,
}

impl DepReport {
    pub fn check(deps: &[Dep]) -> Self {
        Self {
            statuses: deps.iter().map(Dep::check).collect(),
        }
    }

    pub fn all_ok(&self) -> bool {
        self.statuses.iter().all(|s| s.ok)
    }

    pub fn missing(&self) -> impl Iterator<Item = &DepStatus> {
        self.statuses.iter().filter(|s| !s.ok)
    }

    pub fn needs_root(&self) -> bool {
        self.statuses.iter().any(|s| s.dep == Dep::Root && !s.ok)
    }
}

fn hwmon_present(name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
        return false;
    };
    entries.flatten().any(|e| {
        std::fs::read_to_string(e.path().join("name"))
            .map(|n| n.trim() == name)
            .unwrap_or(false)
    })
}

/// Map a binary to the package that provides it on common distros.
fn install_hint(bin: &str) -> String {
    let package = match bin {
        "stress-ng" => "stress-ng",
        "fio" => "fio",
        "nvme" => "nvme-cli",
        "smartctl" => "smartmontools",
        "mkfs.btrfs" | "btrfs" => "btrfs-progs",
        "dmidecode" => "dmidecode",
        "lspci" => "pciutils",
        "sensors" => "lm_sensors",
        other => other,
    };
    match distro_family().as_deref() {
        Some("arch") => format!("sudo pacman -S {package}"),
        Some("fedora") => format!("sudo dnf install {package}"),
        Some("debian") => format!("sudo apt install {package}"),
        _ => format!("install package {package}"),
    }
}

/// Coarse distro family from `/etc/os-release`, used only for install hints.
fn distro_family() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let mut id = None;
    let mut id_like = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            id = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            id_like = Some(v.trim_matches('"').to_string());
        }
    }
    let combined = format!("{} {}", id.unwrap_or_default(), id_like.unwrap_or_default());
    for family in ["arch", "fedora", "debian"] {
        if combined.split_whitespace().any(|w| {
            w == family
                || (family == "debian" && w == "ubuntu")
                || (family == "fedora" && w == "rhel")
        }) {
            return Some(family.to_string());
        }
    }
    None
}
