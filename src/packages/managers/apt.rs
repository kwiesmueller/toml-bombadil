//! APT package manager implementation (Debian/Ubuntu).

use crate::core::{BombadilError, Result};
use crate::packages::managers::{command_exists, PackageManager};
use std::process::Command;
use tracing::debug;

/// APT package manager for Debian, Ubuntu, and related distributions.
#[derive(Debug, Default)]
pub struct Apt;

impl Apt {
    /// Creates a new APT package manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl PackageManager for Apt {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn is_available(&self) -> bool {
        let available = command_exists("apt-get");
        debug!(
            manager = "apt",
            available, "Checking package manager availability"
        );
        available
    }

    fn is_installed(&self, package: &str) -> Result<bool> {
        debug!(manager = "apt", package, "Checking if package is installed");

        let output = Command::new("dpkg-query")
            .args(["-W", "-f=${Status}", package])
            .output()
            .map_err(|e| BombadilError::Io {
                context: format!(
                    "Failed to check if package '{}' is installed via dpkg-query",
                    package
                ),
                source: e,
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let installed = output.status.success() && stdout.contains("install ok installed");
        debug!(
            manager = "apt",
            package, installed, "Package installation check complete"
        );

        Ok(installed)
    }

    fn install(&self, package: &str) -> Result<()> {
        debug!(manager = "apt", package, "Installing package");

        // First, update package lists
        let update_output = Command::new("sudo")
            .args(["apt-get", "update"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to update apt package lists".to_string(),
                source: e,
            })?;

        if !update_output.status.success() {
            debug!(
                manager = "apt",
                "apt-get update failed, continuing with install anyway"
            );
        }

        // Install the package
        let output = Command::new("sudo")
            .args(["apt-get", "install", "-y", package])
            .output()
            .map_err(|e| BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "apt".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "apt".to_string(),
                cause: std::io::Error::other(stderr.to_string()),
            });
        }

        debug!(manager = "apt", package, "Package installed successfully");
        Ok(())
    }

    fn remove(&self, package: &str) -> Result<()> {
        debug!(manager = "apt", package, "Removing package");

        let output = Command::new("sudo")
            .args(["apt-get", "remove", "-y", package])
            .output()
            .map_err(|e| BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "apt".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "apt".to_string(),
                cause: std::io::Error::other(stderr.to_string()),
            });
        }

        debug!(manager = "apt", package, "Package removed successfully");
        Ok(())
    }

    fn list_installed(&self) -> Result<Vec<String>> {
        debug!(manager = "apt", "Listing installed packages");

        let output = Command::new("dpkg-query")
            .args(["-W", "-f=${Package}\\n"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to list installed packages via dpkg-query".to_string(),
                source: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::Io {
                context: format!("dpkg-query failed: {}", stderr),
                source: std::io::Error::other(stderr.to_string()),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let packages: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();

        debug!(
            manager = "apt",
            count = packages.len(),
            "Listed installed packages"
        );
        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apt_name() {
        let apt = Apt::new();
        assert_eq!(apt.name(), "apt");
    }

    #[test]
    fn test_apt_is_available() {
        let apt = Apt::new();
        // Just ensure it doesn't panic - actual availability depends on the system
        let _ = apt.is_available();
    }

    #[test]
    #[ignore = "requires apt to be available"]
    fn test_apt_is_installed() {
        let apt = Apt::new();
        if apt.is_available() {
            // coreutils should be installed on any Debian-based system
            let result = apt.is_installed("coreutils");
            assert!(result.is_ok());
        }
    }

    #[test]
    #[ignore = "requires apt to be available"]
    fn test_apt_list_installed() {
        let apt = Apt::new();
        if apt.is_available() {
            let result = apt.list_installed();
            assert!(result.is_ok());
            // Should have at least some packages installed
            assert!(!result.unwrap().is_empty());
        }
    }
}
