//! DNF package manager implementation (Fedora/RHEL/CentOS).

use crate::core::{BombadilError, Result};
use crate::packages::managers::{command_exists, PackageManager};
use std::process::Command;
use tracing::debug;

/// DNF package manager for Fedora, RHEL, CentOS, and related distributions.
#[derive(Debug, Default)]
pub struct Dnf;

impl Dnf {
    /// Creates a new DNF package manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl PackageManager for Dnf {
    fn name(&self) -> &'static str {
        "dnf"
    }

    fn is_available(&self) -> bool {
        let available = command_exists("dnf");
        debug!(manager = "dnf", available, "Checking package manager availability");
        available
    }

    fn is_installed(&self, package: &str) -> Result<bool> {
        debug!(manager = "dnf", package, "Checking if package is installed");

        let output = Command::new("rpm")
            .args(["-q", package])
            .output()
            .map_err(|e| BombadilError::Io {
                context: format!("Failed to check if package '{}' is installed via rpm", package),
                source: e,
            })?;

        let installed = output.status.success();
        debug!(manager = "dnf", package, installed, "Package installation check complete");

        Ok(installed)
    }

    fn install(&self, package: &str) -> Result<()> {
        debug!(manager = "dnf", package, "Installing package");

        let output = Command::new("sudo")
            .args(["dnf", "install", "-y", package])
            .output()
            .map_err(|e| BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "dnf".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "dnf".to_string(),
                cause: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        debug!(manager = "dnf", package, "Package installed successfully");
        Ok(())
    }

    fn remove(&self, package: &str) -> Result<()> {
        debug!(manager = "dnf", package, "Removing package");

        let output = Command::new("sudo")
            .args(["dnf", "remove", "-y", package])
            .output()
            .map_err(|e| BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "dnf".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "dnf".to_string(),
                cause: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        debug!(manager = "dnf", package, "Package removed successfully");
        Ok(())
    }

    fn list_installed(&self) -> Result<Vec<String>> {
        debug!(manager = "dnf", "Listing installed packages");

        let output = Command::new("rpm")
            .args(["-qa", "--qf", "%{NAME}\\n"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to list installed packages via rpm".to_string(),
                source: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::Io {
                context: format!("rpm -qa failed: {}", stderr),
                source: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let packages: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();

        debug!(manager = "dnf", count = packages.len(), "Listed installed packages");
        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dnf_name() {
        let dnf = Dnf::new();
        assert_eq!(dnf.name(), "dnf");
    }

    #[test]
    fn test_dnf_is_available() {
        let dnf = Dnf::new();
        // Just ensure it doesn't panic - actual availability depends on the system
        let _ = dnf.is_available();
    }

    #[test]
    #[ignore = "requires dnf to be available"]
    fn test_dnf_is_installed() {
        let dnf = Dnf::new();
        if dnf.is_available() {
            // bash should be installed on any system
            let result = dnf.is_installed("bash");
            assert!(result.is_ok());
        }
    }

    #[test]
    #[ignore = "requires dnf to be available"]
    fn test_dnf_list_installed() {
        let dnf = Dnf::new();
        if dnf.is_available() {
            let result = dnf.list_installed();
            assert!(result.is_ok());
            // Should have at least some packages installed
            assert!(!result.unwrap().is_empty());
        }
    }
}
