//! Guards watch the live samples and stop a test when a limit is crossed.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::sampler::Sample;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Guards {
    /// Abort the running test when the watched sensor reaches this (Celsius).
    pub temp_max_c: Option<f32>,
    /// Abort a single test after this many seconds, regardless of its own timeout.
    pub time_max_secs: Option<u64>,
    /// Stop the whole sequence after the first failure or abort.
    pub stop_on_fail: bool,
    /// Pause between tests so temperatures settle.
    pub cooldown_secs: u64,
    /// Abort when a kernel log line matches one of these (case-insensitive substrings).
    pub dmesg_patterns: Vec<String>,
}

impl Default for Guards {
    fn default() -> Self {
        Self {
            temp_max_c: Some(90.0),
            time_max_secs: None,
            stop_on_fail: true,
            cooldown_secs: 10,
            dmesg_patterns: vec![
                "mce:".into(),
                "machine check".into(),
                "aer:".into(),
                "csum".into(),
                "i/o error".into(),
            ],
        }
    }
}

/// Why a guard stopped a test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Trip {
    TempExceeded { temp_c: f32, cap_c: f32 },
    TimeExceeded { elapsed_secs: u64, cap_secs: u64 },
    DmesgMatch { line: String },
    UserCancelled,
}

impl Trip {
    pub fn describe(&self) -> String {
        match self {
            Self::TempExceeded { temp_c, cap_c } => {
                format!("temperature {temp_c:.1}°C reached cap {cap_c:.0}°C")
            }
            Self::TimeExceeded {
                elapsed_secs,
                cap_secs,
            } => format!("ran {elapsed_secs}s, cap {cap_secs}s"),
            Self::DmesgMatch { line } => format!("kernel log: {line}"),
            Self::UserCancelled => "cancelled by user".to_string(),
        }
    }
}

impl Guards {
    pub fn check_sample(&self, sample: &Sample) -> Option<Trip> {
        let cap = self.temp_max_c?;
        let temp = sample.primary_temp_c?;
        (temp >= cap).then_some(Trip::TempExceeded {
            temp_c: temp,
            cap_c: cap,
        })
    }

    pub fn check_elapsed(&self, elapsed: Duration) -> Option<Trip> {
        let cap = self.time_max_secs?;
        let elapsed_secs = elapsed.as_secs();
        (elapsed_secs >= cap).then_some(Trip::TimeExceeded {
            elapsed_secs,
            cap_secs: cap,
        })
    }

    pub fn check_dmesg(&self, line: &str) -> Option<Trip> {
        let lower = line.to_ascii_lowercase();
        self.dmesg_patterns
            .iter()
            .any(|p| !p.is_empty() && lower.contains(&p.to_ascii_lowercase()))
            .then(|| Trip::DmesgMatch {
                line: line.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(temp: f32) -> Sample {
        Sample {
            elapsed_secs: 0.0,
            primary_temp_c: Some(temp),
            temps: vec![],
            freq_mhz: None,
            cpu_percent: None,
            load1: 0.0,
            mem_used_bytes: 0,
        }
    }

    #[test]
    fn temperature_cap_trips_at_or_above_limit() {
        let guards = Guards {
            temp_max_c: Some(90.0),
            ..Default::default()
        };
        assert!(guards.check_sample(&sample(89.9)).is_none());
        assert!(matches!(
            guards.check_sample(&sample(90.0)),
            Some(Trip::TempExceeded { .. })
        ));
    }

    #[test]
    fn no_cap_never_trips() {
        let guards = Guards {
            temp_max_c: None,
            ..Default::default()
        };
        assert!(guards.check_sample(&sample(150.0)).is_none());
    }

    #[test]
    fn time_cap_and_dmesg_patterns() {
        let guards = Guards {
            time_max_secs: Some(60),
            ..Default::default()
        };
        assert!(guards.check_elapsed(Duration::from_secs(59)).is_none());
        assert!(guards.check_elapsed(Duration::from_secs(60)).is_some());
        assert!(
            guards
                .check_dmesg("[12.3] mce: [Hardware Error]: CPU 0")
                .is_some()
        );
        assert!(guards.check_dmesg("[12.3] usb 1-1: new device").is_none());
    }
}
