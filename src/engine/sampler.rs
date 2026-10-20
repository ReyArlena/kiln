//! Periodic sensor sampling: temperatures, CPU frequency, load and memory.
//!
//! Everything is read from sysfs and procfs directly so no extra tools are
//! needed on a bare live ISO.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use super::event::Event;
use super::util::read_trimmed;

/// One temperature input discovered under `/sys/class/hwmon`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorInfo {
    /// Chip name, e.g. `k10temp`.
    pub chip: String,
    /// Input label, e.g. `Tctl`, or `temp1` when the driver gives none.
    pub label: String,
    pub path: PathBuf,
}

impl SensorInfo {
    pub fn id(&self) -> String {
        format!("{}/{}", self.chip, self.label)
    }

    pub fn read_c(&self) -> Option<f32> {
        read_trimmed(&self.path)?
            .parse::<f32>()
            .ok()
            .map(|milli| milli / 1000.0)
    }
}

/// Discover every temperature sensor on the system.
pub fn discover_sensors() -> Vec<SensorInfo> {
    let mut found = Vec::new();
    let Ok(chips) = std::fs::read_dir("/sys/class/hwmon") else {
        return found;
    };
    for chip_dir in chips.flatten().map(|e| e.path()) {
        let Some(chip) = read_trimmed(chip_dir.join("name")) else {
            continue;
        };
        let Ok(files) = std::fs::read_dir(&chip_dir) else {
            continue;
        };
        let mut inputs: Vec<PathBuf> = files
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                name.starts_with("temp") && name.ends_with("_input")
            })
            .collect();
        inputs.sort();
        for path in inputs {
            let stem = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .trim_end_matches("_input")
                .to_string();
            let label = read_trimmed(chip_dir.join(format!("{stem}_label")))
                .unwrap_or_else(|| stem.clone());
            found.push(SensorInfo {
                chip: chip.clone(),
                label,
                path,
            });
        }
    }
    found
}

/// Pick the CPU package sensor: AMD `k10temp/Tctl`, then Intel `coretemp/Package id 0`,
/// then anything from those chips, then the first sensor at all.
pub fn pick_primary(sensors: &[SensorInfo], preferred: Option<&str>) -> Option<SensorInfo> {
    if let Some(id) = preferred
        && let Some(s) = sensors.iter().find(|s| s.id() == id)
    {
        return Some(s.clone());
    }
    let by_id = |id: &str| sensors.iter().find(|s| s.id() == id).cloned();
    by_id("k10temp/Tctl")
        .or_else(|| by_id("coretemp/Package id 0"))
        .or_else(|| {
            sensors
                .iter()
                .find(|s| s.chip == "k10temp" || s.chip == "coretemp" || s.chip == "zenpower")
                .cloned()
        })
        .or_else(|| sensors.first().cloned())
}

/// A point-in-time reading of the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Seconds since the run started.
    pub elapsed_secs: f64,
    /// The watched sensor, used by the temperature guard.
    pub primary_temp_c: Option<f32>,
    /// Every discovered sensor, as `(id, celsius)`.
    pub temps: Vec<(String, f32)>,
    /// Average `scaling_cur_freq` across CPUs, in MHz.
    pub freq_mhz: Option<u32>,
    /// Overall CPU busy percentage since the previous sample.
    pub cpu_percent: Option<f32>,
    /// One-minute load average.
    pub load1: f32,
    pub mem_used_bytes: u64,
    /// CPU package power from RAPL, watts. Needs root on most kernels.
    pub cpu_power_w: Option<f32>,
    /// GPU power from its hwmon `power1_input`, watts.
    pub gpu_power_w: Option<f32>,
}

/// Reads sensors on demand. Cheap to clone; holds only paths.
#[derive(Debug, Clone)]
pub struct Sampler {
    pub sensors: Vec<SensorInfo>,
    pub primary: Option<SensorInfo>,
    /// `(busy, total)` jiffies from the previous read, for CPU percentage.
    prev_cpu: Option<(u64, u64)>,
    /// RAPL package energy counter, when readable.
    rapl_energy: Option<PathBuf>,
    /// Energy (microjoules) and time of the previous read, for CPU watts.
    prev_energy: Option<(u64, Instant)>,
    /// GPU hwmon power input, when present.
    gpu_power: Option<PathBuf>,
}

impl Sampler {
    pub fn new(preferred: Option<&str>) -> Self {
        let sensors = discover_sensors();
        let primary = pick_primary(&sensors, preferred);
        Self {
            sensors,
            primary,
            prev_cpu: None,
            rapl_energy: find_rapl_package(),
            prev_energy: None,
            gpu_power: find_gpu_power(),
        }
    }

    pub fn read(&mut self, started: Instant) -> Sample {
        let temps = self
            .sensors
            .iter()
            .filter_map(|s| s.read_c().map(|c| (s.id(), c)))
            .collect();
        Sample {
            elapsed_secs: started.elapsed().as_secs_f64(),
            primary_temp_c: self.primary.as_ref().and_then(SensorInfo::read_c),
            temps,
            freq_mhz: average_cpu_freq_mhz(),
            cpu_percent: self.cpu_percent(),
            load1: load_average(),
            mem_used_bytes: mem_used_bytes(),
            cpu_power_w: self.cpu_power_w(),
            gpu_power_w: self
                .gpu_power
                .as_ref()
                .and_then(|p| read_trimmed(p)?.parse::<f64>().ok())
                .map(|uw| (uw / 1_000_000.0) as f32),
        }
    }

    /// Average package power since the last read, from the RAPL energy counter.
    fn cpu_power_w(&mut self) -> Option<f32> {
        let path = self.rapl_energy.as_ref()?;
        let energy_uj: u64 = read_trimmed(path)?.parse().ok()?;
        let now = Instant::now();
        let previous = self.prev_energy.replace((energy_uj, now))?;
        let secs = now.duration_since(previous.1).as_secs_f64();
        if secs <= 0.0 || energy_uj < previous.0 {
            return None; // counter wrapped
        }
        Some(((energy_uj - previous.0) as f64 / 1_000_000.0 / secs) as f32)
    }

    pub fn has_cpu_power(&self) -> bool {
        self.rapl_energy.is_some()
    }

    pub fn has_gpu_power(&self) -> bool {
        self.gpu_power.is_some()
    }

    /// Busy share of all CPUs since the last call. `None` on the first call.
    fn cpu_percent(&mut self) -> Option<f32> {
        let (busy, total) = cpu_jiffies()?;
        let previous = self.prev_cpu.replace((busy, total))?;
        let d_total = total.saturating_sub(previous.1);
        if d_total == 0 {
            return None;
        }
        Some(busy.saturating_sub(previous.0) as f32 * 100.0 / d_total as f32)
    }

    /// Emit a sample every `interval` until `stop` flips to true.
    pub async fn run(
        mut self,
        interval: Duration,
        started: Instant,
        tx: mpsc::UnboundedSender<Event>,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if tx.send(Event::Sample(self.read(started))).is_err() {
                        return;
                    }
                }
                _ = stop.changed() => {
                    if *stop.borrow() {
                        return;
                    }
                }
            }
        }
    }
}

fn average_cpu_freq_mhz() -> Option<u32> {
    let Ok(cpus) = std::fs::read_dir("/sys/devices/system/cpu") else {
        return None;
    };
    let mut total_khz = 0u64;
    let mut count = 0u64;
    for entry in cpus.flatten() {
        let name = entry.file_name();
        let name = name.to_str().unwrap_or("");
        if !name.starts_with("cpu") || !name[3..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(khz) = read_trimmed(entry.path().join("cpufreq/scaling_cur_freq"))
            .and_then(|s| s.parse::<u64>().ok())
        {
            total_khz += khz;
            count += 1;
        }
    }
    (count > 0).then(|| (total_khz / count / 1000) as u32)
}

/// The RAPL package energy counter, only if this process can read it
/// (kernels make it root-only).
fn find_rapl_package() -> Option<PathBuf> {
    let zones = std::fs::read_dir("/sys/class/powercap").ok()?;
    let mut candidates: Vec<PathBuf> = zones
        .flatten()
        .map(|e| e.path())
        .filter(|p| read_trimmed(p.join("name")).is_some_and(|n| n.starts_with("package")))
        .map(|p| p.join("energy_uj"))
        .filter(|p| std::fs::read_to_string(p).is_ok())
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// `power1_input` (or `power1_average`) of a GPU hwmon chip, in microwatts.
fn find_gpu_power() -> Option<PathBuf> {
    let chips = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for dir in chips.flatten().map(|e| e.path()) {
        let name = read_trimmed(dir.join("name")).unwrap_or_default();
        if name != "amdgpu" && name != "nouveau" && name != "nvidia" {
            continue;
        }
        for file in ["power1_input", "power1_average"] {
            let path = dir.join(file);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

/// `(busy, total)` jiffies from the aggregate line of `/proc/stat`.
fn cpu_jiffies() -> Option<(u64, u64)> {
    let text = read_trimmed("/proc/stat")?;
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

fn load_average() -> f32 {
    read_trimmed("/proc/loadavg")
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0)
}

fn mem_used_bytes() -> u64 {
    let Some(text) = read_trimmed("/proc/meminfo") else {
        return 0;
    };
    let field = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    field("MemTotal:").saturating_sub(field("MemAvailable:"))
}

/// Total physical memory from `/proc/meminfo`, in bytes.
pub fn mem_total_bytes() -> u64 {
    read_trimmed("/proc/meminfo")
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .unwrap_or(0)
        * 1024
}

/// Whether a given sysfs path exists, exposed for tests and doctor output.
pub fn sysfs_exists(path: &str) -> bool {
    Path::new(path).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensor(chip: &str, label: &str) -> SensorInfo {
        SensorInfo {
            chip: chip.into(),
            label: label.into(),
            path: PathBuf::from("/nonexistent"),
        }
    }

    #[test]
    fn primary_sensor_prefers_cpu_package() {
        let sensors = vec![
            sensor("nvme", "Composite"),
            sensor("k10temp", "Tctl"),
            sensor("k10temp", "Tccd1"),
        ];
        assert_eq!(pick_primary(&sensors, None).unwrap().id(), "k10temp/Tctl");
        assert_eq!(
            pick_primary(&sensors, Some("nvme/Composite")).unwrap().id(),
            "nvme/Composite"
        );
        assert_eq!(
            pick_primary(&sensors, Some("missing/x")).unwrap().id(),
            "k10temp/Tctl"
        );
        assert!(pick_primary(&[], None).is_none());
    }
}
