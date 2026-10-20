//! Resolves the directories kiln reads and writes.
//!
//! Layout follows the XDG convention used by most Linux apps:
//!
//! - `~/.config/kiln/config.toml`            app settings
//! - `~/.config/kiln/profiles/<test>/*.toml`  saved parameter sets per test
//! - `~/.config/kiln/plans/*.toml`            saved multi-test plans
//! - `~/.local/share/kiln/runs/<run-id>/`     results and logs for each run
//! - `~/.local/share/kiln/kiln.log`           application log
//!
//! Most tests need root, so kiln is usually launched through `sudo`. In that
//! case `$HOME` points at `/root`, which would scatter settings between two
//! homes. When `SUDO_USER` is set the invoking user's home is used instead,
//! and files kiln creates there are handed back to that user.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use nix::unistd::{Gid, Uid, User};

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub profiles_dir: PathBuf,
    pub plans_dir: PathBuf,
    pub data_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub log_file: PathBuf,
    /// Owner to apply to files kiln creates, when running under sudo.
    owner: Option<(Uid, Gid)>,
}

impl Paths {
    /// Work out every path from the environment. Nothing is created yet.
    pub fn resolve() -> Result<Self> {
        let sudo = sudo_invoker()?;

        let (config_dir, data_dir) = match &sudo {
            Some((home, _, _)) => (home.join(".config/kiln"), home.join(".local/share/kiln")),
            None => {
                let dirs = ProjectDirs::from("", "", "kiln")
                    .context("could not determine a home directory")?;
                (
                    dirs.config_dir().to_path_buf(),
                    dirs.data_dir().to_path_buf(),
                )
            }
        };

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            profiles_dir: config_dir.join("profiles"),
            plans_dir: config_dir.join("plans"),
            runs_dir: data_dir.join("runs"),
            log_file: data_dir.join("kiln.log"),
            config_dir,
            data_dir,
            owner: sudo.map(|(_, uid, gid)| (uid, gid)),
        })
    }

    /// Everything under one base directory. Used by tests and for sandboxed runs.
    pub fn under(base: &Path) -> Self {
        let config_dir = base.join("config");
        let data_dir = base.join("data");
        Self {
            config_file: config_dir.join("config.toml"),
            profiles_dir: config_dir.join("profiles"),
            plans_dir: config_dir.join("plans"),
            runs_dir: data_dir.join("runs"),
            log_file: data_dir.join("kiln.log"),
            config_dir,
            data_dir,
            owner: None,
        }
    }

    /// Create the directory tree if it is missing.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.profiles_dir,
            &self.plans_dir,
            &self.data_dir,
            &self.runs_dir,
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            self.restore_owner(dir);
        }
        Ok(())
    }

    /// Hand a file or directory back to the sudo invoker. Best effort: a
    /// failure here only means the file stays root-owned.
    pub fn restore_owner(&self, path: &Path) {
        if let Some((uid, gid)) = self.owner {
            let _ = nix::unistd::chown(path, Some(uid), Some(gid));
        }
    }

    /// Directory holding saved profiles for one test.
    pub fn profiles_for(&self, test_id: &str) -> PathBuf {
        self.profiles_dir.join(test_id)
    }
}

/// When launched with sudo, returns the invoking user's home, uid and gid.
fn sudo_invoker() -> Result<Option<(PathBuf, Uid, Gid)>> {
    let Ok(name) = std::env::var("SUDO_USER") else {
        return Ok(None);
    };
    let user = User::from_name(&name)
        .with_context(|| format!("looking up SUDO_USER {name}"))?
        .with_context(|| format!("SUDO_USER {name} does not exist"))?;
    Ok(Some((user.dir, user.uid, user.gid)))
}
