//! Package management for Bombadil.
//!
//! This module provides functionality for installing, tracking, and managing
//! software packages across different platforms and installation methods.

pub mod discover;
pub mod drift;
pub mod install;
pub mod managers;
pub mod persist;
pub mod state;

use crate::platform::PlatformContext;
use crate::settings::packages::{Package, PackageManagerConfig};
use anyhow::{anyhow, Result};
use colored::*;
use std::collections::HashMap;
use std::path::PathBuf;

/// Manages package installation and state
pub struct PackageManager {
    /// All configured packages
    packages: HashMap<String, Package>,
    /// Platform context for OS detection
    platform: PlatformContext,
    /// Path to dotfiles directory
    dotfiles_path: PathBuf,
    /// Tag filters for installation
    include_tags: Vec<String>,
    /// Tags to exclude from installation
    exclude_tags: Vec<String>,
}

impl PackageManager {
    /// Create a new package manager
    pub fn new(
        packages: HashMap<String, Package>,
        dotfiles_path: PathBuf,
        include_tags: Vec<String>,
        exclude_tags: Vec<String>,
    ) -> Self {
        Self {
            packages,
            platform: PlatformContext::detect(),
            dotfiles_path,
            include_tags,
            exclude_tags,
        }
    }

    /// Install all packages matching the configured tag filters
    pub fn install_all(&self) -> Result<Vec<InstallResult>> {
        let mut results = Vec::new();

        for (name, package) in &self.packages {
            if !package.enabled {
                println!("{} {} (disabled)", "Skipping".yellow(), name);
                continue;
            }

            if !package.matches_tags(&self.include_tags, &self.exclude_tags) {
                println!("{} {} (tags don't match filter)", "Skipping".yellow(), name);
                continue;
            }

            let result = self.install_package(name, package)?;
            results.push(result);
        }

        Ok(results)
    }

    /// Install a specific package by name
    pub fn install_by_name(&self, name: &str) -> Result<InstallResult> {
        match self.packages.get(name) {
            Some(package) => self.install_package(name, package),
            None => Ok(InstallResult {
                name: name.to_string(),
                status: InstallStatus::NotFound,
                method: None,
                version: None,
            }),
        }
    }

    /// Check if a package is known (exists in configuration)
    pub fn is_known(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// Get a package by name
    pub fn get_package(&self, name: &str) -> Option<&Package> {
        self.packages.get(name)
    }

    /// Install a single package
    fn install_package(&self, name: &str, package: &Package) -> Result<InstallResult> {
        println!("{} {}...", "Installing".green().bold(), name.bold());

        // Select the best installation method for this platform
        let method = self.select_install_method(package)?;

        let install_result = match method.as_str() {
            "dnf" => self.install_with_dnf(name, package),
            "apt" => self.install_with_apt(name, package),
            "brew" => self.install_with_brew(name, package),
            "pacman" => self.install_with_pacman(name, package),
            "cargo" => self.install_with_cargo(name, package),
            "go" => self.install_with_go(name, package),
            "flatpak" => self.install_with_flatpak(name, package),
            "binary" => self.install_binary(name, package),
            "git" => self.install_from_git(name, package),
            "source" => self.install_from_source(name, package),
            _ => Err(anyhow!("Unknown install method: {}", method)),
        };

        match install_result {
            Ok(version) => {
                println!("  {} {} installed successfully", "✓".green(), name);
                Ok(InstallResult {
                    name: name.to_string(),
                    status: InstallStatus::Installed,
                    method: Some(method),
                    version,
                })
            }
            Err(e) => {
                eprintln!("  {} Failed to install {}: {}", "✗".red(), name, e);
                Ok(InstallResult {
                    name: name.to_string(),
                    status: InstallStatus::Failed(e.to_string()),
                    method: Some(method),
                    version: None,
                })
            }
        }
    }

    /// Select the best installation method based on platform and available methods
    fn select_install_method(&self, package: &Package) -> Result<String> {
        let methods = &package.install;

        // Platform-specific package manager preference
        match self.platform.os.as_str() {
            "linux" => {
                if let Some(distro) = &self.platform.distro {
                    match distro.as_str() {
                        "fedora" | "rhel" | "centos" | "rocky" | "alma" => {
                            if methods.dnf.is_some() {
                                return Ok("dnf".to_string());
                            }
                        }
                        "ubuntu" | "debian" | "pop" | "mint" | "elementary" => {
                            if methods.apt.is_some() {
                                return Ok("apt".to_string());
                            }
                        }
                        "arch" | "manjaro" | "endeavouros" => {
                            if methods.pacman.is_some() {
                                return Ok("pacman".to_string());
                            }
                        }
                        _ => {}
                    }
                }
                // Linux fallback: try brew if available
                if methods.brew.is_some() && self.has_command("brew") {
                    return Ok("brew".to_string());
                }
            }
            "macos" => {
                if methods.brew.is_some() {
                    return Ok("brew".to_string());
                }
            }
            _ => {}
        }

        // Language-specific package managers
        if methods.cargo.is_some() && self.has_command("cargo") {
            return Ok("cargo".to_string());
        }
        if methods.go.is_some() && self.has_command("go") {
            return Ok("go".to_string());
        }

        // Flatpak
        if methods.flatpak.is_some() && self.has_command("flatpak") {
            return Ok("flatpak".to_string());
        }

        // Binary download (always available)
        if methods.binary.is_some() {
            return Ok("binary".to_string());
        }

        // Git clone
        if methods.git.is_some() && self.has_command("git") {
            return Ok("git".to_string());
        }

        // Local source
        if methods.source.is_some() {
            return Ok("source".to_string());
        }

        Err(anyhow!(
            "No suitable installation method found for this platform"
        ))
    }

    /// Check if a command is available in PATH
    fn has_command(&self, cmd: &str) -> bool {
        std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    // Installation method implementations

    fn install_with_dnf(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .dnf
            .as_ref()
            .ok_or_else(|| anyhow!("No dnf config"))?;
        let pkg_name = config.package_name();

        // Handle repo setup if needed
        if let PackageManagerConfig::Extended {
            repo,
            repo_url,
            gpg_key,
            ..
        } = config
        {
            if let Some(repo_file) = repo {
                let repo_path = self.dotfiles_path.join(repo_file);
                if repo_path.exists() {
                    install::run_command(
                        "sudo",
                        &["cp", repo_path.to_str().unwrap(), "/etc/yum.repos.d/"],
                    )?;
                }
            }
            if let Some(key_url) = gpg_key {
                install::run_command("sudo", &["rpm", "--import", key_url])?;
            }
            if let Some(url) = repo_url {
                install::run_command("sudo", &["dnf", "config-manager", "--add-repo", url])?;
            }
        }

        install::run_command("sudo", &["dnf", "install", "-y", pkg_name])?;
        Ok(None) // TODO: Get installed version
    }

    fn install_with_apt(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .apt
            .as_ref()
            .ok_or_else(|| anyhow!("No apt config"))?;
        let pkg_name = config.package_name();

        // Handle repo setup if needed
        if let PackageManagerConfig::Extended {
            repo_url, gpg_key, ..
        } = config
        {
            if let Some(key_url) = gpg_key {
                install::run_command_piped(
                    &["curl", "-fsSL", key_url],
                    &[
                        "sudo",
                        "gpg",
                        "--dearmor",
                        "-o",
                        "/usr/share/keyrings/package.gpg",
                    ],
                )?;
            }
            if let Some(url) = repo_url {
                install::run_command("sudo", &["add-apt-repository", "-y", url])?;
            }
        }

        install::run_command("sudo", &["apt-get", "update"])?;
        install::run_command("sudo", &["apt-get", "install", "-y", pkg_name])?;
        Ok(None)
    }

    fn install_with_brew(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .brew
            .as_ref()
            .ok_or_else(|| anyhow!("No brew config"))?;
        let pkg_name = config.package_name();

        install::run_command("brew", &["install", pkg_name])?;
        Ok(None)
    }

    fn install_with_pacman(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .pacman
            .as_ref()
            .ok_or_else(|| anyhow!("No pacman config"))?;
        let pkg_name = config.package_name();

        install::run_command("sudo", &["pacman", "-S", "--noconfirm", pkg_name])?;
        Ok(None)
    }

    fn install_with_cargo(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .cargo
            .as_ref()
            .ok_or_else(|| anyhow!("No cargo config"))?;

        let args = config.to_args();
        let mut cmd_args = vec!["install"];
        cmd_args.extend(args.iter().map(|s| s.as_str()));

        install::run_command("cargo", &cmd_args)?;
        Ok(None)
    }

    fn install_with_go(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let pkg_path = package
            .install
            .go
            .as_ref()
            .ok_or_else(|| anyhow!("No go config"))?;

        install::run_command("go", &["install", pkg_path])?;
        Ok(None)
    }

    fn install_with_flatpak(&self, _name: &str, package: &Package) -> Result<Option<String>> {
        let app_id = package
            .install
            .flatpak
            .as_ref()
            .ok_or_else(|| anyhow!("No flatpak config"))?;

        install::run_command("flatpak", &["install", "-y", "flathub", app_id])?;
        Ok(None)
    }

    fn install_binary(&self, name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .binary
            .as_ref()
            .ok_or_else(|| anyhow!("No binary config"))?;

        install::install_binary(name, config, &self.platform)
    }

    fn install_from_git(&self, name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .git
            .as_ref()
            .ok_or_else(|| anyhow!("No git config"))?;

        install::install_from_git(name, config, &self.dotfiles_path)
    }

    fn install_from_source(&self, name: &str, package: &Package) -> Result<Option<String>> {
        let config = package
            .install
            .source
            .as_ref()
            .ok_or_else(|| anyhow!("No source config"))?;

        install::install_from_source(name, config)
    }

    /// Remove a specific package by name using the appropriate package manager
    pub fn remove_package(&self, name: &str) -> Result<RemoveResult> {
        let package = match self.packages.get(name) {
            Some(pkg) => pkg,
            None => {
                return Ok(RemoveResult {
                    name: name.to_string(),
                    status: RemoveStatus::NotFound,
                    method: None,
                })
            }
        };

        println!("{} {}...", "Removing".red().bold(), name.bold());

        let method = self.select_install_method(package)?;

        let remove_result = match method.as_str() {
            "dnf" => self.remove_with_dnf(name, package),
            "apt" => self.remove_with_apt(name, package),
            "brew" => self.remove_with_brew(name, package),
            "pacman" => self.remove_with_pacman(name, package),
            "cargo" => self.remove_with_cargo(name, package),
            "flatpak" => self.remove_with_flatpak(name, package),
            _ => Err(anyhow!("Removal not supported for method: {}", method)),
        };

        match remove_result {
            Ok(()) => {
                println!("  {} {} removed successfully", "✓".green(), name);
                Ok(RemoveResult {
                    name: name.to_string(),
                    status: RemoveStatus::Removed,
                    method: Some(method),
                })
            }
            Err(e) => {
                eprintln!("  {} Failed to remove {}: {}", "✗".red(), name, e);
                Ok(RemoveResult {
                    name: name.to_string(),
                    status: RemoveStatus::Failed(e.to_string()),
                    method: Some(method),
                })
            }
        }
    }

    fn remove_with_dnf(&self, _name: &str, package: &Package) -> Result<()> {
        let config = package
            .install
            .dnf
            .as_ref()
            .ok_or_else(|| anyhow!("No dnf config"))?;
        let pkg_name = config.package_name();

        install::run_command("sudo", &["dnf", "remove", "-y", pkg_name])?;
        Ok(())
    }

    fn remove_with_apt(&self, _name: &str, package: &Package) -> Result<()> {
        let config = package
            .install
            .apt
            .as_ref()
            .ok_or_else(|| anyhow!("No apt config"))?;
        let pkg_name = config.package_name();

        install::run_command("sudo", &["apt-get", "remove", "-y", pkg_name])?;
        Ok(())
    }

    fn remove_with_brew(&self, _name: &str, package: &Package) -> Result<()> {
        let config = package
            .install
            .brew
            .as_ref()
            .ok_or_else(|| anyhow!("No brew config"))?;
        let pkg_name = config.package_name();

        install::run_command("brew", &["uninstall", pkg_name])?;
        Ok(())
    }

    fn remove_with_pacman(&self, _name: &str, package: &Package) -> Result<()> {
        let config = package
            .install
            .pacman
            .as_ref()
            .ok_or_else(|| anyhow!("No pacman config"))?;
        let pkg_name = config.package_name();

        install::run_command("sudo", &["pacman", "-R", "--noconfirm", pkg_name])?;
        Ok(())
    }

    fn remove_with_cargo(&self, _name: &str, package: &Package) -> Result<()> {
        let config = package
            .install
            .cargo
            .as_ref()
            .ok_or_else(|| anyhow!("No cargo config"))?;
        let pkg_name = config.package_name();

        install::run_command("cargo", &["uninstall", pkg_name])?;
        Ok(())
    }

    fn remove_with_flatpak(&self, _name: &str, package: &Package) -> Result<()> {
        let app_id = package
            .install
            .flatpak
            .as_ref()
            .ok_or_else(|| anyhow!("No flatpak config"))?;

        install::run_command("flatpak", &["uninstall", "-y", app_id])?;
        Ok(())
    }

    /// List all packages with their status
    pub fn list_packages(&self) -> Vec<PackageInfo> {
        self.packages
            .iter()
            .map(|(name, pkg)| PackageInfo {
                name: name.clone(),
                tags: pkg.tags.clone(),
                enabled: pkg.enabled,
                available_methods: pkg.install.available_methods(),
            })
            .collect()
    }
}

/// Result of a package installation attempt
#[derive(Debug)]
pub struct InstallResult {
    pub name: String,
    pub status: InstallStatus,
    pub method: Option<String>,
    pub version: Option<String>,
}

/// Status of a package installation
#[derive(Debug)]
pub enum InstallStatus {
    Installed,
    AlreadyInstalled,
    Failed(String),
    NotFound,
    Skipped,
}

/// Result of a package removal attempt
#[derive(Debug)]
pub struct RemoveResult {
    pub name: String,
    pub status: RemoveStatus,
    pub method: Option<String>,
}

/// Status of a package removal
#[derive(Debug)]
pub enum RemoveStatus {
    Removed,
    Failed(String),
    NotFound,
}

/// Information about a configured package
#[derive(Debug)]
pub struct PackageInfo {
    pub name: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub available_methods: Vec<&'static str>,
}
