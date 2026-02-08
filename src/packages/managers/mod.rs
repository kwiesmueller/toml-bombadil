//! Package manager abstractions for Bombadil v4.
//!
//! This module provides a unified trait for interacting with different package
//! managers across various operating systems and installation methods.

pub mod apt;
pub mod brew;
pub mod cargo;
pub mod dnf;
pub mod flatpak;
pub mod pacman;

use crate::core::Result;

/// Trait defining the interface for package manager implementations.
///
/// Each package manager (dnf, apt, brew, etc.) implements this trait to provide
/// a consistent API for checking, installing, and listing packages.
pub trait PackageManager: Send + Sync {
    /// Returns the name of this package manager (e.g., "dnf", "apt", "brew").
    fn name(&self) -> &'static str;

    /// Checks if this package manager is available on the current system.
    ///
    /// This typically checks if the package manager binary exists in PATH.
    fn is_available(&self) -> bool;

    /// Checks if a specific package is installed.
    ///
    /// # Arguments
    /// * `package` - The name of the package to check
    ///
    /// # Returns
    /// * `Ok(true)` if the package is installed
    /// * `Ok(false)` if the package is not installed
    /// * `Err(_)` if the check failed
    fn is_installed(&self, package: &str) -> Result<bool>;

    /// Installs a package.
    ///
    /// # Arguments
    /// * `package` - The name of the package to install
    ///
    /// # Returns
    /// * `Ok(())` if installation succeeded
    /// * `Err(_)` if installation failed
    fn install(&self, package: &str) -> Result<()>;

    /// Removes a package.
    ///
    /// # Arguments
    /// * `package` - The name of the package to remove
    ///
    /// # Returns
    /// * `Ok(())` if removal succeeded
    /// * `Err(_)` if removal failed
    fn remove(&self, package: &str) -> Result<()>;

    /// Lists all installed packages.
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of installed package names
    /// * `Err(_)` if listing failed
    fn list_installed(&self) -> Result<Vec<String>>;
}

/// Helper function to check if a command exists in PATH.
pub fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_exists_with_common_command() {
        // 'ls' should exist on any Unix-like system
        assert!(command_exists("ls"));
    }

    #[test]
    fn test_command_exists_with_nonexistent_command() {
        assert!(!command_exists("this_command_definitely_does_not_exist_12345"));
    }
}
