//! Flatpak package manager implementation.

use crate::core::{BombadilError, Result};
use crate::packages::managers::{command_exists, PackageManager};
use std::process::Command;
use tracing::debug;

/// Flatpak package manager for sandboxed applications.
#[derive(Debug, Default)]
pub struct Flatpak;

impl Flatpak {
    /// Creates a new Flatpak package manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl PackageManager for Flatpak {
    fn name(&self) -> &'static str {
        "flatpak"
    }

    fn is_available(&self) -> bool {
        let available = command_exists("flatpak");
        debug!(manager = "flatpak", available, "Checking package manager availability");
        available
    }

    fn is_installed(&self, package: &str) -> Result<bool> {
        debug!(manager = "flatpak", package, "Checking if package is installed");

        // Flatpak uses application IDs like "org.gnome.Calculator"
        let output = Command::new("flatpak")
            .args(["info", package])
            .output()
            .map_err(|e| BombadilError::Io {
                context: format!("Failed to check if flatpak '{}' is installed", package),
                source: e,
            })?;

        let installed = output.status.success();
        debug!(manager = "flatpak", package, installed, "Package installation check complete");

        Ok(installed)
    }

    fn install(&self, package: &str) -> Result<()> {
        debug!(manager = "flatpak", package, "Installing package");

        // Install from Flathub by default
        let output = Command::new("flatpak")
            .args(["install", "-y", "flathub", package])
            .output()
            .map_err(|e| BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "flatpak".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "flatpak".to_string(),
                cause: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        debug!(manager = "flatpak", package, "Package installed successfully");
        Ok(())
    }

    fn remove(&self, package: &str) -> Result<()> {
        debug!(manager = "flatpak", package, "Removing package");

        let output = Command::new("flatpak")
            .args(["uninstall", "-y", package])
            .output()
            .map_err(|e| BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "flatpak".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "flatpak".to_string(),
                cause: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        debug!(manager = "flatpak", package, "Package removed successfully");
        Ok(())
    }

    fn list_installed(&self) -> Result<Vec<String>> {
        debug!(manager = "flatpak", "Listing installed packages");

        let output = Command::new("flatpak")
            .args(["list", "--app", "--columns=application"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to list installed flatpaks".to_string(),
                source: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::Io {
                context: format!("flatpak list failed: {}", stderr),
                source: std::io::Error::new(std::io::ErrorKind::Other, stderr.to_string()),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let packages: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            // Skip the header line if present
            .filter(|line| *line != "Application ID")
            .map(|line| line.to_string())
            .collect();

        debug!(manager = "flatpak", count = packages.len(), "Listed installed packages");
        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatpak_name() {
        let flatpak = Flatpak::new();
        assert_eq!(flatpak.name(), "flatpak");
    }

    #[test]
    fn test_flatpak_is_available() {
        let flatpak = Flatpak::new();
        // Just ensure it doesn't panic - actual availability depends on the system
        let _ = flatpak.is_available();
    }

    #[test]
    #[ignore = "requires flatpak to be available"]
    fn test_flatpak_is_installed_nonexistent() {
        let flatpak = Flatpak::new();
        if flatpak.is_available() {
            let result = flatpak.is_installed("com.nonexistent.app.12345");
            assert!(result.is_ok());
            assert!(!result.unwrap());
        }
    }

    #[test]
    #[ignore = "requires flatpak to be available"]
    fn test_flatpak_list_installed() {
        let flatpak = Flatpak::new();
        if flatpak.is_available() {
            let result = flatpak.list_installed();
            assert!(result.is_ok());
            // The list might be empty, but the call should succeed
        }
    }
}
