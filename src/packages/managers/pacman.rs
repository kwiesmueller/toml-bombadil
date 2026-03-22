//! Pacman package manager implementation (Arch Linux).

use crate::core::{BombadilError, Result};
use crate::packages::managers::{command_exists, PackageManager};
use std::process::Command;
use tracing::debug;

/// Pacman package manager for Arch Linux and related distributions.
#[derive(Debug, Default)]
pub struct Pacman;

impl Pacman {
    /// Creates a new Pacman package manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl PackageManager for Pacman {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn is_available(&self) -> bool {
        let available = command_exists("pacman");
        debug!(
            manager = "pacman",
            available, "Checking package manager availability"
        );
        available
    }

    fn is_installed(&self, package: &str) -> Result<bool> {
        debug!(
            manager = "pacman",
            package, "Checking if package is installed"
        );

        let output = Command::new("pacman")
            .args(["-Q", package])
            .output()
            .map_err(|e| BombadilError::Io {
                context: format!(
                    "Failed to check if package '{}' is installed via pacman",
                    package
                ),
                source: e,
            })?;

        let installed = output.status.success();
        debug!(
            manager = "pacman",
            package, installed, "Package installation check complete"
        );

        Ok(installed)
    }

    fn install(&self, package: &str) -> Result<()> {
        debug!(manager = "pacman", package, "Installing package");

        let output = Command::new("sudo")
            .args(["pacman", "-S", "--noconfirm", package])
            .output()
            .map_err(|e| BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "pacman".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "pacman".to_string(),
                cause: std::io::Error::other(stderr.to_string()),
            });
        }

        debug!(
            manager = "pacman",
            package, "Package installed successfully"
        );
        Ok(())
    }

    fn remove(&self, package: &str) -> Result<()> {
        debug!(manager = "pacman", package, "Removing package");

        let output = Command::new("sudo")
            .args(["pacman", "-R", "--noconfirm", package])
            .output()
            .map_err(|e| BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "pacman".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "pacman".to_string(),
                cause: std::io::Error::other(stderr.to_string()),
            });
        }

        debug!(manager = "pacman", package, "Package removed successfully");
        Ok(())
    }

    fn list_installed(&self) -> Result<Vec<String>> {
        debug!(manager = "pacman", "Listing installed packages");

        let output =
            Command::new("pacman")
                .args(["-Qq"])
                .output()
                .map_err(|e| BombadilError::Io {
                    context: "Failed to list installed packages via pacman".to_string(),
                    source: e,
                })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::Io {
                context: format!("pacman -Qq failed: {}", stderr),
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
            manager = "pacman",
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
    fn test_pacman_name() {
        let pacman = Pacman::new();
        assert_eq!(pacman.name(), "pacman");
    }

    #[test]
    fn test_pacman_is_available() {
        let pacman = Pacman::new();
        // Just ensure it doesn't panic - actual availability depends on the system
        let _ = pacman.is_available();
    }

    #[test]
    #[ignore = "requires pacman to be available"]
    fn test_pacman_is_installed() {
        let pacman = Pacman::new();
        if pacman.is_available() {
            // bash should be installed on any Arch system
            let result = pacman.is_installed("bash");
            assert!(result.is_ok());
        }
    }

    #[test]
    #[ignore = "requires pacman to be available"]
    fn test_pacman_list_installed() {
        let pacman = Pacman::new();
        if pacman.is_available() {
            let result = pacman.list_installed();
            assert!(result.is_ok());
            // Should have at least some packages installed
            assert!(!result.unwrap().is_empty());
        }
    }
}
