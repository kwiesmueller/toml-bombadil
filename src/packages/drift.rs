//! Package drift detection (v4).
//!
//! Compares packages declared in the v4 config against what is actually
//! installed on the system, reporting mismatches.

use crate::config::Config;
use crate::packages::managers::PackageManager;
use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;

/// Report of drift between configured and installed packages.
pub struct DriftReport {
    /// Packages declared in config but not currently installed.
    pub missing: Vec<String>,
    /// Packages installed on the system but not declared in config.
    /// Only populated by [`DriftReport::detect_with_extras`].
    pub extra: Vec<String>,
}

impl DriftReport {
    /// Detect packages that are declared in config but not installed.
    ///
    /// Does not list packages installed outside the config (use
    /// [`detect_with_extras`](Self::detect_with_extras) for that).
    pub fn detect(config: &Config, managers: &[Box<dyn PackageManager>]) -> Result<Self> {
        let managed: HashSet<String> = config.settings.packages.keys().cloned().collect();

        let available: Vec<&dyn PackageManager> = managers
            .iter()
            .filter(|m| m.is_available())
            .map(|m| m.as_ref())
            .collect();

        let mut missing = Vec::new();
        for name in &managed {
            let installed = available
                .iter()
                .any(|m| m.is_installed(name).unwrap_or(false));
            if !installed {
                missing.push(name.clone());
            }
        }
        missing.sort();

        Ok(DriftReport {
            missing,
            extra: vec![],
        })
    }

    /// Detect drift in both directions: missing (config but not installed)
    /// and extra (installed but not in config).
    pub fn detect_with_extras(config: &Config, managers: &[Box<dyn PackageManager>]) -> Result<Self> {
        let managed: HashSet<String> = config.settings.packages.keys().cloned().collect();

        let available: Vec<&dyn PackageManager> = managers
            .iter()
            .filter(|m| m.is_available())
            .map(|m| m.as_ref())
            .collect();

        let mut missing = Vec::new();
        for name in &managed {
            let installed = available
                .iter()
                .any(|m| m.is_installed(name).unwrap_or(false));
            if !installed {
                missing.push(name.clone());
            }
        }
        missing.sort();

        let mut all_installed: HashSet<String> = HashSet::new();
        for mgr in &available {
            if let Ok(pkgs) = mgr.list_installed() {
                all_installed.extend(pkgs);
            }
        }

        let mut extra: Vec<String> = all_installed
            .difference(&managed)
            .cloned()
            .collect();
        extra.sort();

        Ok(DriftReport { missing, extra })
    }

    /// Print the drift report to stdout.
    pub fn print(&self) {
        if self.missing.is_empty() && self.extra.is_empty() {
            println!("{}", "No package drift detected.".green());
            return;
        }

        if !self.missing.is_empty() {
            println!("{}", "Missing (in config but not installed):".yellow().bold());
            for name in &self.missing {
                println!("  {} {}", "-".red(), name);
            }
        }

        if !self.extra.is_empty() {
            println!("{}", "Extra (installed but not in config):".cyan().bold());
            for name in &self.extra {
                println!("  {} {}", "+".green(), name);
            }
        }
    }

    /// Returns `true` if there is no drift.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }
}
