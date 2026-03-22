//! Cargo package manager implementation (Rust crates).

use crate::core::{BombadilError, Result};
use crate::packages::managers::{command_exists, PackageManager};
use std::process::Command;
use tracing::debug;

/// Cargo package manager for Rust crates.
#[derive(Debug, Default)]
pub struct Cargo;

impl Cargo {
    /// Creates a new Cargo package manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl PackageManager for Cargo {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn is_available(&self) -> bool {
        let available = command_exists("cargo");
        debug!(
            manager = "cargo",
            available, "Checking package manager availability"
        );
        available
    }

    fn is_installed(&self, package: &str) -> Result<bool> {
        debug!(
            manager = "cargo",
            package, "Checking if package is installed"
        );

        // Use cargo install --list to check for installed packages
        let output = Command::new("cargo")
            .args(["install", "--list"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: format!(
                    "Failed to check if crate '{}' is installed via cargo",
                    package
                ),
                source: e,
            })?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Cargo install --list output format:
        // package_name v1.0.0:
        //     binary_name
        // The package name is at the start of a line followed by a version
        let installed = stdout.lines().any(|line| {
            let trimmed = line.trim();
            // Lines starting without whitespace are package entries
            if line.starts_with(char::is_whitespace) {
                return false;
            }
            // Check if the line starts with the package name
            trimmed.starts_with(package)
                && (trimmed.len() == package.len()
                    || trimmed.chars().nth(package.len()) == Some(' '))
        });

        debug!(
            manager = "cargo",
            package, installed, "Package installation check complete"
        );
        Ok(installed)
    }

    fn install(&self, package: &str) -> Result<()> {
        debug!(manager = "cargo", package, "Installing package");

        let output = Command::new("cargo")
            .args(["install", package])
            .output()
            .map_err(|e| BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "cargo".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageInstallFailed {
                name: package.to_string(),
                manager: "cargo".to_string(),
                cause: std::io::Error::other(stderr.to_string()),
            });
        }

        debug!(manager = "cargo", package, "Package installed successfully");
        Ok(())
    }

    fn remove(&self, package: &str) -> Result<()> {
        debug!(manager = "cargo", package, "Removing package");

        let output = Command::new("cargo")
            .args(["uninstall", package])
            .output()
            .map_err(|e| BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "cargo".to_string(),
                cause: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::PackageRemoveFailed {
                name: package.to_string(),
                manager: "cargo".to_string(),
                cause: std::io::Error::other(stderr.to_string()),
            });
        }

        debug!(manager = "cargo", package, "Package removed successfully");
        Ok(())
    }

    fn list_installed(&self) -> Result<Vec<String>> {
        debug!(manager = "cargo", "Listing installed packages");

        let output = Command::new("cargo")
            .args(["install", "--list"])
            .output()
            .map_err(|e| BombadilError::Io {
                context: "Failed to list installed crates via cargo".to_string(),
                source: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BombadilError::Io {
                context: format!("cargo install --list failed: {}", stderr),
                source: std::io::Error::other(stderr.to_string()),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let packages: Vec<String> = stdout
            .lines()
            .filter(|line| {
                // Only include lines that don't start with whitespace (package names)
                !line.is_empty() && !line.starts_with(char::is_whitespace)
            })
            .filter_map(|line| {
                // Extract package name (before the version)
                line.split_whitespace().next().map(|s| s.to_string())
            })
            .collect();

        debug!(
            manager = "cargo",
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
    fn test_cargo_name() {
        let cargo = Cargo::new();
        assert_eq!(cargo.name(), "cargo");
    }

    #[test]
    fn test_cargo_is_available() {
        let cargo = Cargo::new();
        // Cargo should be available in the development environment
        let available = cargo.is_available();
        // Don't assert, just ensure it doesn't panic
        debug!(available, "Cargo availability check");
    }

    #[test]
    fn test_cargo_list_installed() {
        let cargo = Cargo::new();
        if cargo.is_available() {
            let result = cargo.list_installed();
            assert!(result.is_ok());
            // The list might be empty, but the call should succeed
        }
    }

    #[test]
    fn test_cargo_is_installed_nonexistent() {
        let cargo = Cargo::new();
        if cargo.is_available() {
            let result = cargo.is_installed("this_crate_definitely_does_not_exist_12345");
            assert!(result.is_ok());
            assert!(!result.unwrap());
        }
    }
}
