//! Homebrew package manager implementation (macOS/Linux).

use crate::core::{BombadilError, Result};
use crate::packages::managers::{command_exists, PackageManager};
use std::process::Command;
use tracing::debug;

/// Homebrew package manager for macOS and Linux.
#[derive(Debug, Default)]
pub struct Brew;

impl Brew {
    /// Creates a new Homebrew package manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl PackageManager for Brew {
    fn name(&self) -> &'static str {
        "brew"
    }

    fn is_available(&self) -> bool {
        let available = command_exists("brew");
        debug!(manager = "brew", available, "Checking package manager availability");
        available
    }

    fn is_installed(&self, package: &str) -> Result<bool> {
        debug!(manager = "brew", package, "Checking if package is installed");

        let output = Command::new("brew")
            .args(["list", "--formula", package])
            .output()
            .map_err(|e| BombadilError::Io {
                context: format!("Failed to check if package '{}' is installed via brew", package),
                source: e,
            })?;

        // If formula check fails, try cask
        let installed = if output.status.success() {
            true
        } else {
            let cask_output = Command::new("brew")
                .args(["list", "--cask", package])
                .output()
                .map_err(|e| BombadilError::Io {
                    context: format!("Failed to check if cask '{}' is installed via brew", package),
                    source: e,
                })?;
            cask_output.status.success()
        };

        debug!(manager = "brew", package, installed, "Package installation check complete");
        Ok(installed)
    }

    fn install(&self, package: &str) -> Result<()> {
        debug!(manager = "brew", package, "Installing package");

        let output = Command::new("brew")
            .args(["install", package])
            .output()
            .map_err(|e| BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "brew".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "brew".to_string(),
                cause: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        debug!(manager = "brew", package, "Package installed successfully");
        Ok(())
    }

    fn remove(&self, package: &str) -> Result<()> {
        debug!(manager = "brew", package, "Removing package");

        let output = Command::new("brew")
            .args(["uninstall", package])
            .output()
            .map_err(|e| BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "brew".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "brew".to_string(),
                cause: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        debug!(manager = "brew", package, "Package removed successfully");
        Ok(())
    }

    fn list_installed(&self) -> Result<Vec<String>> {
        debug!(manager = "brew", "Listing installed packages");

        // Get installed formulae
        let formula_output = Command::new("brew")
            .args(["list", "--formula", "-1"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to list installed formulae via brew".to_string(),
                source: e,
            })?;

        let mut packages: Vec<String> = Vec::new();

        if formula_output.status.success() {
            let stdout = String::from_utf8_lossy(&formula_output.stdout);
            packages.extend(
                stdout
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string()),
            );
        }

        // Get installed casks
        let cask_output = Command::new("brew")
            .args(["list", "--cask", "-1"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to list installed casks via brew".to_string(),
                source: e,
            })?;

        if cask_output.status.success() {
            let stdout = String::from_utf8_lossy(&cask_output.stdout);
            packages.extend(
                stdout
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string()),
            );
        }

        debug!(manager = "brew", count = packages.len(), "Listed installed packages");
        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brew_name() {
        let brew = Brew::new();
        assert_eq!(brew.name(), "brew");
    }

    #[test]
    fn test_brew_is_available() {
        let brew = Brew::new();
        // Just ensure it doesn't panic - actual availability depends on the system
        let _ = brew.is_available();
    }

    #[test]
    #[ignore = "requires brew to be available"]
    fn test_brew_is_installed() {
        let brew = Brew::new();
        if brew.is_available() {
            // Test with a package that is typically installed if brew is available
            let result = brew.is_installed("git");
            assert!(result.is_ok());
        }
    }

    #[test]
    #[ignore = "requires brew to be available"]
    fn test_brew_list_installed() {
        let brew = Brew::new();
        if brew.is_available() {
            let result = brew.list_installed();
            assert!(result.is_ok());
        }
    }
}
