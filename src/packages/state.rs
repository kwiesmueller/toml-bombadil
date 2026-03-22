//! State tracking for installed packages.
//!
//! This module manages the persistent state of installed packages,
//! tracking versions, install methods, and timestamps.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const PACKAGES_STATE_FILE: &str = "packages_state.toml";

/// State of all installed packages
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct PackagesState {
    /// Path to state file (not serialized)
    #[serde(skip)]
    pub path: PathBuf,

    /// Map of package name to installed state
    #[serde(default)]
    pub packages: HashMap<String, InstalledPackage>,
}

/// State of a single installed package
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InstalledPackage {
    /// Version that was installed (if known)
    #[serde(default)]
    pub version: Option<String>,

    /// Installation method used
    pub method: String,

    /// Target path (for binary installs)
    #[serde(default)]
    pub target: Option<PathBuf>,

    /// Timestamp when installed
    pub installed_at: DateTime<Utc>,

    /// Timestamp of last update check
    #[serde(default)]
    pub last_checked: Option<DateTime<Utc>>,
}

impl PackagesState {
    /// Read packages state from the dotfiles directory
    pub fn read(dotfiles_path: &Path) -> Result<Self> {
        let state_path = dotfiles_path.join(".dots").join(PACKAGES_STATE_FILE);

        if state_path.exists() {
            let content = fs::read_to_string(&state_path)
                .with_context(|| format!("reading packages state {}", state_path.display()))?;
            let mut state: PackagesState =
                toml::from_str(&content).with_context(|| "Failed to parse packages state")?;

            state.path = state_path;
            Ok(state)
        } else {
            Ok(Self {
                path: state_path,
                packages: HashMap::new(),
            })
        }
    }

    /// Write state to file
    pub fn write(&self) -> Result<()> {
        // Ensure .dots directory exists
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content =
            toml::to_string_pretty(&self).context("Failed to serialize packages state")?;
        fs::write(&self.path, content).context("Failed to write packages state")?;

        Ok(())
    }

    /// Record a package installation
    pub fn record_install(
        &mut self,
        name: &str,
        method: &str,
        version: Option<String>,
        target: Option<PathBuf>,
    ) {
        self.packages.insert(
            name.to_string(),
            InstalledPackage {
                version,
                method: method.to_string(),
                target,
                installed_at: Utc::now(),
                last_checked: Some(Utc::now()),
            },
        );
    }

    /// Remove a package from state
    pub fn remove(&mut self, name: &str) -> Option<InstalledPackage> {
        self.packages.remove(name)
    }

    /// Check if a package is installed
    pub fn is_installed(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// Get installed package info
    pub fn get(&self, name: &str) -> Option<&InstalledPackage> {
        self.packages.get(name)
    }

    /// Update the version for a package
    pub fn update_version(&mut self, name: &str, version: String) {
        if let Some(pkg) = self.packages.get_mut(name) {
            pkg.version = Some(version);
            pkg.last_checked = Some(Utc::now());
        }
    }

    /// Get packages that were installed but are no longer in config
    pub fn orphaned_packages<'a>(&'a self, configured: &[String]) -> Vec<&'a str> {
        self.packages
            .keys()
            .filter(|name| !configured.contains(name))
            .map(|s| s.as_str())
            .collect()
    }

    /// Get packages that are in config but not installed
    pub fn missing_packages<'a>(&self, configured: &'a [String]) -> Vec<&'a str> {
        configured
            .iter()
            .filter(|name| !self.packages.contains_key(*name))
            .map(|s| s.as_str())
            .collect()
    }
}

impl InstalledPackage {
    /// Check if the package might need an update (based on last check time)
    pub fn needs_update_check(&self, max_age_hours: i64) -> bool {
        match self.last_checked {
            Some(last) => {
                let age = Utc::now() - last;
                age.num_hours() > max_age_hours
            }
            None => true,
        }
    }

    /// Format as display string
    pub fn display(&self) -> String {
        let version_str = self.version.as_deref().unwrap_or("unknown");
        let target_str = self
            .target
            .as_ref()
            .map(|p| format!(" -> {}", p.display()))
            .unwrap_or_default();

        format!(
            "{} via {} (installed {}){}",
            version_str,
            self.method,
            self.installed_at.format("%Y-%m-%d"),
            target_str
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_roundtrip() {
        let temp_dir = tempdir().unwrap();
        let dotfiles_path = temp_dir.path();

        // Create state
        let mut state = PackagesState::read(dotfiles_path).unwrap();
        state.record_install("ripgrep", "cargo", Some("14.0.0".to_string()), None);
        state.record_install(
            "sops",
            "binary",
            Some("3.8.1".to_string()),
            Some(PathBuf::from("/home/user/.local/bin/sops")),
        );
        state.write().unwrap();

        // Read back
        let state2 = PackagesState::read(dotfiles_path).unwrap();
        assert_eq!(state2.packages.len(), 2);
        assert!(state2.is_installed("ripgrep"));
        assert!(state2.is_installed("sops"));

        let rg = state2.get("ripgrep").unwrap();
        assert_eq!(rg.version, Some("14.0.0".to_string()));
        assert_eq!(rg.method, "cargo");
    }

    #[test]
    fn test_orphaned_packages() {
        let mut state = PackagesState::default();
        state.record_install("pkg1", "dnf", None, None);
        state.record_install("pkg2", "dnf", None, None);
        state.record_install("pkg3", "dnf", None, None);

        let configured = vec!["pkg1".to_string(), "pkg3".to_string()];
        let orphaned = state.orphaned_packages(&configured);

        assert_eq!(orphaned, vec!["pkg2"]);
    }

    #[test]
    fn test_missing_packages() {
        let mut state = PackagesState::default();
        state.record_install("pkg1", "dnf", None, None);

        let configured = vec!["pkg1".to_string(), "pkg2".to_string(), "pkg3".to_string()];
        let mut missing = state.missing_packages(&configured);
        missing.sort();

        assert_eq!(missing, vec!["pkg2", "pkg3"]);
    }
}
