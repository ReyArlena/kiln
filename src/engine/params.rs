//! Test parameters: the schema a test publishes and the values a user picks.
//!
//! A [`ParamSpec`] is what the TUI form and the CLI `--set key=value` flag are
//! generated from. A [`ParamSet`] is the resolved map of values passed to the
//! test when it builds its job. Saved profiles are simply serialised sets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::util::{format_bytes, format_duration_secs, parse_bytes, parse_duration_secs};

/// One parameter a test accepts, with enough detail to build a form field.
#[derive(Debug, Clone)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: ParamKind,
}

#[derive(Debug, Clone)]
pub enum ParamKind {
    /// Wall-clock duration, stored in seconds.
    Duration {
        default: u64,
        min: u64,
        max: u64,
    },
    /// Plain integer with bounds. `default` of zero with `zero_means_auto`
    /// lets a test substitute something sensible (e.g. CPU count).
    Int {
        default: i64,
        min: i64,
        max: i64,
        zero_means_auto: bool,
    },
    /// Byte size, entered like `20G` or `512M`.
    Bytes {
        default: u64,
    },
    /// One of a fixed list of strings.
    Choice {
        choices: &'static [&'static str],
        default: &'static str,
    },
    Bool {
        default: bool,
    },
    /// A path that must exist.
    Path {
        default: &'static str,
    },
    /// A whole block device that the test will overwrite. Never has a
    /// default: the user must type it.
    BlockDevice,
}

impl ParamSpec {
    /// The value used when the user has not set anything.
    pub fn default_value(&self) -> Option<ParamValue> {
        Some(match &self.kind {
            ParamKind::Duration { default, .. } => ParamValue::Duration(*default),
            ParamKind::Int { default, .. } => ParamValue::Int(*default),
            ParamKind::Bytes { default } => ParamValue::Bytes(*default),
            ParamKind::Choice { default, .. } => ParamValue::Text((*default).to_string()),
            ParamKind::Bool { default } => ParamValue::Bool(*default),
            ParamKind::Path { default } => ParamValue::Text((*default).to_string()),
            ParamKind::BlockDevice => return None,
        })
    }

    /// Parse user-typed text into a value and validate it against the spec.
    pub fn parse(&self, text: &str) -> Result<ParamValue, ParamError> {
        let text = text.trim();
        let bad = |why: &str| ParamError::Invalid {
            key: self.key.to_string(),
            why: why.to_string(),
        };
        match &self.kind {
            ParamKind::Duration { min, max, .. } => {
                let secs = parse_duration_secs(text)
                    .ok_or_else(|| bad("expected a duration like 90, 5m or 1h30m"))?;
                if secs < *min || secs > *max {
                    return Err(bad(&format!(
                        "must be between {} and {}",
                        format_duration_secs(*min),
                        format_duration_secs(*max)
                    )));
                }
                Ok(ParamValue::Duration(secs))
            }
            ParamKind::Int { min, max, .. } => {
                let value: i64 = text.parse().map_err(|_| bad("expected a whole number"))?;
                if value < *min || value > *max {
                    return Err(bad(&format!("must be between {min} and {max}")));
                }
                Ok(ParamValue::Int(value))
            }
            ParamKind::Bytes { .. } => {
                let bytes =
                    parse_bytes(text).ok_or_else(|| bad("expected a size like 20G or 512M"))?;
                if bytes == 0 {
                    return Err(bad("must be greater than zero"));
                }
                Ok(ParamValue::Bytes(bytes))
            }
            ParamKind::Choice { choices, .. } => {
                if !choices.contains(&text) {
                    return Err(bad(&format!("must be one of: {}", choices.join(", "))));
                }
                Ok(ParamValue::Text(text.to_string()))
            }
            ParamKind::Bool { .. } => match text.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Ok(ParamValue::Bool(true)),
                "false" | "no" | "off" | "0" => Ok(ParamValue::Bool(false)),
                _ => Err(bad("expected true or false")),
            },
            ParamKind::Path { .. } => {
                if !std::path::Path::new(text).exists() {
                    return Err(bad("path does not exist"));
                }
                Ok(ParamValue::Text(text.to_string()))
            }
            ParamKind::BlockDevice => {
                if !text.starts_with("/dev/") {
                    return Err(bad("expected a device path under /dev"));
                }
                Ok(ParamValue::Text(text.to_string()))
            }
        }
    }

    /// Validate a value that was loaded from a profile rather than typed.
    pub fn validate(&self, value: &ParamValue) -> Result<(), ParamError> {
        self.parse(&value.to_text()).map(|_| ())
    }

    pub fn is_destructive(&self) -> bool {
        matches!(self.kind, ParamKind::BlockDevice)
    }
}

/// A concrete parameter value. Serialises to plain TOML/JSON scalars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Duration(u64),
    Bytes(u64),
    Text(String),
}

impl ParamValue {
    /// Text form used for display and for feeding commands.
    pub fn to_text(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Duration(secs) => secs.to_string(),
            Self::Bytes(b) => format_bytes(*b),
            Self::Text(s) => s.clone(),
        }
    }

    /// Human-friendly form for the UI (durations get units, sizes get suffixes).
    pub fn to_display(&self) -> String {
        match self {
            Self::Duration(secs) => format_duration_secs(*secs),
            other => other.to_text(),
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Int(i) => u64::try_from(*i).ok(),
            Self::Duration(v) | Self::Bytes(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Duration(v) | Self::Bytes(v) => i64::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Resolved parameter values keyed by spec key.
pub type ParamSet = BTreeMap<String, ParamValue>;

/// Build a set from a spec list: defaults, then profile values, then overrides.
/// Values from profiles and overrides are re-validated so a stale file cannot
/// smuggle in something out of range.
pub fn resolve_params(specs: &[ParamSpec], layers: &[&ParamSet]) -> Result<ParamSet, ParamError> {
    let mut set = ParamSet::new();
    for spec in specs {
        if let Some(value) = spec.default_value() {
            set.insert(spec.key.to_string(), value);
        }
    }
    for layer in layers {
        for (key, value) in layer.iter() {
            let spec = specs
                .iter()
                .find(|s| s.key == key)
                .ok_or_else(|| ParamError::Unknown { key: key.clone() })?;
            // Loaded values may have lost their variant (untagged serde), so
            // normalise through the spec's parser.
            let parsed = spec.parse(&value.to_text())?;
            set.insert(key.clone(), parsed);
        }
    }
    for spec in specs {
        if !set.contains_key(spec.key) {
            return Err(ParamError::Missing {
                key: spec.key.to_string(),
            });
        }
    }
    Ok(set)
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ParamError {
    #[error("parameter `{key}`: {why}")]
    Invalid { key: String, why: String },
    #[error("unknown parameter `{key}`")]
    Unknown { key: String },
    #[error("parameter `{key}` is required")]
    Missing { key: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ParamSpec> {
        vec![
            ParamSpec {
                key: "duration",
                label: "Duration",
                help: "",
                kind: ParamKind::Duration {
                    default: 60,
                    min: 1,
                    max: 3600,
                },
            },
            ParamSpec {
                key: "workers",
                label: "Workers",
                help: "",
                kind: ParamKind::Int {
                    default: 0,
                    min: 0,
                    max: 512,
                    zero_means_auto: true,
                },
            },
            ParamSpec {
                key: "device",
                label: "Device",
                help: "",
                kind: ParamKind::BlockDevice,
            },
        ]
    }

    #[test]
    fn defaults_then_layers_then_validation() {
        let specs = specs();
        let mut profile = ParamSet::new();
        profile.insert("duration".into(), ParamValue::Text("5m".into()));
        let mut overrides = ParamSet::new();
        overrides.insert("device".into(), ParamValue::Text("/dev/sdz".into()));

        let set = resolve_params(&specs, &[&profile, &overrides]).unwrap();
        assert_eq!(set["duration"], ParamValue::Duration(300));
        assert_eq!(set["workers"], ParamValue::Int(0));
        assert_eq!(set["device"].as_str(), Some("/dev/sdz"));
    }

    #[test]
    fn required_device_must_be_supplied() {
        let err = resolve_params(&specs(), &[]).unwrap_err();
        assert_eq!(
            err,
            ParamError::Missing {
                key: "device".into()
            }
        );
    }

    #[test]
    fn out_of_range_and_unknown_keys_are_rejected() {
        let specs = specs();
        let mut bad = ParamSet::new();
        bad.insert("workers".into(), ParamValue::Int(9999));
        assert!(matches!(
            resolve_params(&specs, &[&bad]),
            Err(ParamError::Invalid { .. })
        ));

        let mut unknown = ParamSet::new();
        unknown.insert("nope".into(), ParamValue::Bool(true));
        assert!(matches!(
            resolve_params(&specs, &[&unknown]),
            Err(ParamError::Unknown { .. })
        ));
    }
}
