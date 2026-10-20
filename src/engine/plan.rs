//! A plan is an ordered list of tests with guards. The picker produces one,
//! `kiln run` builds one from flags, and plans can be saved as TOML.
//!
//! Profiles are saved parameter sets for a single test. A plan entry can name
//! a profile and still override individual values on top.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::guards::Guards;
use super::params::ParamSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub name: String,
    #[serde(default)]
    pub guards: Guards,
    #[serde(default)]
    pub tests: Vec<PlanEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub test: String,
    /// Named profile under `profiles/<test>/`. `None` means the built-in default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Values layered over the profile.
    #[serde(default, skip_serializing_if = "ParamSet::is_empty")]
    pub params: ParamSet,
    /// Run this entry this many times back to back.
    #[serde(default = "one")]
    pub repeat: u32,
}

fn one() -> u32 {
    1
}

impl PlanEntry {
    pub fn new(test: &str) -> Self {
        Self {
            test: test.to_string(),
            profile: None,
            params: ParamSet::new(),
            repeat: 1,
        }
    }
}

impl Plan {
    pub fn new(name: &str, guards: Guards) -> Self {
        Self {
            name: name.to_string(),
            guards,
            tests: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// A saved parameter set for one test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: ParamSet,
}

impl Profile {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.toml", self.name));
        std::fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Load every profile saved for a test, sorted by name. Missing dir is fine.
    pub fn list(dir: &Path) -> Vec<Profile> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut profiles: Vec<Profile> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .filter_map(|p| Profile::load(&p).ok())
            .collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        profiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::params::ParamValue;

    #[test]
    fn plan_round_trips_through_toml() {
        let mut plan = Plan::new("nightly", Guards::default());
        let mut entry = PlanEntry::new("cpu-stress");
        entry
            .params
            .insert("duration".into(), ParamValue::Duration(600));
        entry.repeat = 2;
        plan.tests.push(entry);

        let text = toml::to_string(&plan).unwrap();
        let back: Plan = toml::from_str(&text).unwrap();
        assert_eq!(back.tests[0].test, "cpu-stress");
        assert_eq!(back.tests[0].repeat, 2);
        // Untagged values come back as plain integers; the runner re-validates them.
        assert_eq!(back.tests[0].params["duration"].as_u64(), Some(600));
    }
}
