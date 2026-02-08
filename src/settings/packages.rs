//! Package configuration types for declarative package management.
//!
//! This module defines the configuration format for packages that can be
//! installed via various methods (system package managers, cargo, go, binaries, etc.)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A package definition with multiple installation methods and metadata
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Package {
    /// Tags for categorizing and filtering packages (e.g., ["cli", "dev", "desktop"])
    #[serde(default)]
    pub tags: Vec<String>,

    /// Installation methods for different platforms/tools
    #[serde(default)]
    pub install: InstallMethods,

    /// Whether this package is enabled (can be overridden by profiles)
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Collection of installation methods for a package
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct InstallMethods {
    /// DNF package manager (Fedora, RHEL)
    #[serde(default)]
    pub dnf: Option<PackageManagerConfig>,

    /// APT package manager (Debian, Ubuntu)
    #[serde(default)]
    pub apt: Option<PackageManagerConfig>,

    /// Homebrew package manager (macOS, Linux)
    #[serde(default)]
    pub brew: Option<PackageManagerConfig>,

    /// Pacman package manager (Arch Linux)
    #[serde(default)]
    pub pacman: Option<PackageManagerConfig>,

    /// Cargo install (Rust packages)
    #[serde(default)]
    pub cargo: Option<CargoConfig>,

    /// Go install (Go packages)
    #[serde(default)]
    pub go: Option<String>,

    /// Binary download from URL or GitHub release
    #[serde(default)]
    pub binary: Option<BinaryConfig>,

    /// Flatpak application
    #[serde(default)]
    pub flatpak: Option<String>,

    /// Git repository to clone and build
    #[serde(default)]
    pub git: Option<GitConfig>,

    /// Local source directory to build from
    #[serde(default)]
    pub source: Option<SourceConfig>,
}

/// Configuration for system package managers (dnf, apt, brew, pacman)
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum PackageManagerConfig {
    /// Simple package name
    Simple(String),
    /// Extended configuration with repo setup
    Extended {
        /// Package name to install
        package: String,
        /// Repository file to install (copied to appropriate location)
        #[serde(default)]
        repo: Option<String>,
        /// Repository URL to add
        #[serde(default)]
        repo_url: Option<String>,
        /// GPG key URL for repository
        #[serde(default)]
        gpg_key: Option<String>,
    },
}

impl PackageManagerConfig {
    /// Get the package name from the configuration
    pub fn package_name(&self) -> &str {
        match self {
            PackageManagerConfig::Simple(name) => name,
            PackageManagerConfig::Extended { package, .. } => package,
        }
    }
}

/// Configuration for cargo install
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum CargoConfig {
    /// Simple crate name (from crates.io)
    Simple(String),
    /// Extended configuration
    Extended {
        /// Crate name or git URL (v3 format)
        crate_name: Option<String>,
        /// Crate name (v4 format alias for crate_name)
        #[serde(default)]
        name: Option<String>,
        /// Git repository URL
        #[serde(default)]
        git: Option<String>,
        /// Branch to use (for git installs)
        #[serde(default)]
        branch: Option<String>,
        /// Tag to use (for git installs)
        #[serde(default)]
        tag: Option<String>,
        /// Specific features to enable
        #[serde(default)]
        features: Vec<String>,
        /// Binary name (if different from crate name)
        #[serde(default)]
        bin: Option<String>,
    },
}

impl CargoConfig {
    /// Get the package/crate name from the configuration
    pub fn package_name(&self) -> &str {
        match self {
            CargoConfig::Simple(name) => name,
            CargoConfig::Extended { crate_name, name, bin, .. } => {
                bin.as_deref()
                    .or(crate_name.as_deref())
                    .or(name.as_deref())
                    .unwrap_or("unknown")
            }
        }
    }

    /// Build the cargo install command arguments
    pub fn to_args(&self) -> Vec<String> {
        match self {
            CargoConfig::Simple(name) => vec![name.clone()],
            CargoConfig::Extended {
                crate_name,
                name,
                git,
                branch,
                tag,
                features,
                bin,
            } => {
                let mut args = Vec::new();

                if let Some(git_url) = git {
                    args.push("--git".to_string());
                    args.push(git_url.clone());

                    if let Some(branch) = branch {
                        args.push("--branch".to_string());
                        args.push(branch.clone());
                    }
                    if let Some(tag) = tag {
                        args.push("--tag".to_string());
                        args.push(tag.clone());
                    }
                } else if let Some(pkg_name) = crate_name.as_ref().or(name.as_ref()) {
                    args.push(pkg_name.clone());
                }

                if !features.is_empty() {
                    args.push("--features".to_string());
                    args.push(features.join(","));
                }

                if let Some(bin) = bin {
                    args.push("--bin".to_string());
                    args.push(bin.clone());
                }

                args
            }
        }
    }
}

/// Configuration for binary downloads
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum BinaryConfig {
    /// Direct URL download
    Url {
        /// URL template (supports {{version}}, {{os}}, {{arch}})
        url: String,
        /// Version to download
        #[serde(default)]
        version: Option<String>,
        /// Command to get current installed version
        #[serde(default)]
        version_cmd: Option<String>,
        /// Target path for the binary
        #[serde(default)]
        target: Option<PathBuf>,
        /// Whether the download is an archive to extract
        #[serde(default)]
        archive: Option<ArchiveConfig>,
        /// SHA256 checksum for verification
        #[serde(default)]
        sha256: Option<String>,
    },
    /// GitHub release download
    GitHub {
        /// GitHub repository (owner/repo)
        github: String,
        /// Asset filename pattern (supports {{version}}, {{os}}, {{arch}})
        asset_pattern: String,
        /// Specific version/tag (defaults to latest)
        #[serde(default)]
        version: Option<String>,
        /// Target path for the binary
        #[serde(default)]
        target: Option<PathBuf>,
        /// Whether the download is an archive to extract
        #[serde(default)]
        archive: Option<ArchiveConfig>,
    },
}

/// Configuration for archive extraction
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ArchiveConfig {
    /// Type of archive (tar.gz, zip, tar.xz)
    #[serde(default)]
    pub format: Option<String>,
    /// Path within archive to extract (for single binary)
    #[serde(default)]
    pub binary_path: Option<String>,
}

/// Configuration for git-based installation
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitConfig {
    /// Git repository URL
    pub url: String,
    /// Branch to checkout
    #[serde(default)]
    pub branch: Option<String>,
    /// Tag to checkout
    #[serde(default)]
    pub tag: Option<String>,
    /// Build command to run after clone
    #[serde(default)]
    pub build: Option<String>,
    /// Target directory to clone into
    #[serde(default)]
    pub target: Option<PathBuf>,
}

/// Configuration for local source installation
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SourceConfig {
    /// Path to the source directory
    pub path: PathBuf,
    /// Build command to run
    #[serde(default)]
    pub build: Option<String>,
}

/// Override for packages in profiles (allows enabling/disabling or changing settings)
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct PackageOverride {
    /// Override enabled state
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Additional tags to add
    #[serde(default)]
    pub tags: Vec<String>,

    /// Override installation methods
    #[serde(default)]
    pub install: Option<InstallMethods>,
}

/// Settings for package management behavior
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct PackageSettings {
    /// Tags to include by default (packages must have at least one of these tags)
    #[serde(default)]
    pub package_tags: Vec<String>,

    /// Tags to exclude (packages with any of these tags will be skipped)
    #[serde(default)]
    pub excluded_package_tags: Vec<String>,

    /// Preferred installation method order
    #[serde(default)]
    pub preferred_methods: Vec<String>,
}

impl Package {
    /// Check if the package should be installed based on tag filters
    pub fn matches_tags(&self, include: &[String], exclude: &[String]) -> bool {
        // If exclude tags are specified and package has any, skip it
        if !exclude.is_empty() && self.tags.iter().any(|t| exclude.contains(t)) {
            return false;
        }

        // If include tags are specified, package must have at least one
        if !include.is_empty() {
            return self.tags.iter().any(|t| include.contains(t));
        }

        // If no include tags specified, include all (that aren't excluded)
        true
    }

    /// Create a new package with a simple package manager install method
    pub fn new_with_package_manager(method: &str, package_name: &str, tags: Vec<String>) -> Self {
        let mut install = InstallMethods::default();
        let config = PackageManagerConfig::Simple(package_name.to_string());

        match method {
            "dnf" => install.dnf = Some(config),
            "apt" => install.apt = Some(config),
            "brew" => install.brew = Some(config),
            "pacman" => install.pacman = Some(config),
            _ => {}
        }

        Package {
            tags,
            install,
            enabled: true,
        }
    }

    /// Create a new package with cargo install
    pub fn new_with_cargo(crate_name: &str, tags: Vec<String>) -> Self {
        Package {
            tags,
            install: InstallMethods {
                cargo: Some(CargoConfig::Simple(crate_name.to_string())),
                ..Default::default()
            },
            enabled: true,
        }
    }

    /// Create a new package with go install
    pub fn new_with_go(package_path: &str, tags: Vec<String>) -> Self {
        Package {
            tags,
            install: InstallMethods {
                go: Some(package_path.to_string()),
                ..Default::default()
            },
            enabled: true,
        }
    }

    /// Create a new package with flatpak
    pub fn new_with_flatpak(app_id: &str, tags: Vec<String>) -> Self {
        Package {
            tags,
            install: InstallMethods {
                flatpak: Some(app_id.to_string()),
                ..Default::default()
            },
            enabled: true,
        }
    }

    /// Generate TOML string for this package (for appending to config files)
    pub fn to_toml_string(&self, name: &str) -> String {
        // Use a wrapper to get the [packages.name] format
        let mut packages = HashMap::new();
        packages.insert(name.to_string(), self.clone());

        let wrapper = PackagesWrapper { packages };
        toml::to_string_pretty(&wrapper).unwrap_or_default()
    }
}

/// Wrapper struct for serializing packages in the correct TOML format
#[derive(Serialize)]
struct PackagesWrapper {
    packages: HashMap<String, Package>,
}

/// Template content for package configuration (loaded at compile time)
pub const PACKAGE_TEMPLATE: &str = include_str!("../templates/package.template.toml");

impl Package {
    /// Generate a template TOML string for interactive editing
    pub fn generate_template(name: &str, initial_tags: &[String]) -> String {
        let tags_str = if initial_tags.is_empty() {
            r#""cli""#.to_string()
        } else {
            initial_tags
                .iter()
                .map(|t| format!("\"{}\"", t))
                .collect::<Vec<_>>()
                .join(", ")
        };

        PACKAGE_TEMPLATE
            .replace("{{name}}", name)
            .replace("{{tags}}", &tags_str)
    }
}

impl InstallMethods {
    /// Check if any install method is configured
    pub fn has_any(&self) -> bool {
        self.dnf.is_some()
            || self.apt.is_some()
            || self.brew.is_some()
            || self.pacman.is_some()
            || self.cargo.is_some()
            || self.go.is_some()
            || self.binary.is_some()
            || self.flatpak.is_some()
            || self.git.is_some()
            || self.source.is_some()
    }

    /// Get a list of available install method names
    pub fn available_methods(&self) -> Vec<&'static str> {
        let mut methods = Vec::new();
        if self.dnf.is_some() {
            methods.push("dnf");
        }
        if self.apt.is_some() {
            methods.push("apt");
        }
        if self.brew.is_some() {
            methods.push("brew");
        }
        if self.pacman.is_some() {
            methods.push("pacman");
        }
        if self.cargo.is_some() {
            methods.push("cargo");
        }
        if self.go.is_some() {
            methods.push("go");
        }
        if self.binary.is_some() {
            methods.push("binary");
        }
        if self.flatpak.is_some() {
            methods.push("flatpak");
        }
        if self.git.is_some() {
            methods.push("git");
        }
        if self.source.is_some() {
            methods.push("source");
        }
        methods
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_package_manager_config() {
        let toml = r#"dnf = "ripgrep""#;
        let _methods: InstallMethods = toml::from_str(&format!("[install]\n{}", toml))
            .map(|p: Package| p.install)
            .unwrap_or_default();

        // Parse just the install methods
        let config: HashMap<String, toml::Value> = toml::from_str(toml).unwrap();
        assert!(config.contains_key("dnf"));
    }

    #[test]
    fn test_extended_package_manager_config() {
        let toml = r#"
        [packages.kubectl]
        tags = ["k8s"]
        [packages.kubectl.install]
        dnf = { package = "kubectl", repo = "kubernetes.repo" }
        "#;

        let config: HashMap<String, HashMap<String, Package>> = toml::from_str(toml).unwrap();
        let pkg = config.get("packages").unwrap().get("kubectl").unwrap();

        assert_eq!(pkg.tags, vec!["k8s"]);
        assert!(pkg.install.dnf.is_some());

        if let Some(PackageManagerConfig::Extended { package, repo, .. }) = &pkg.install.dnf {
            assert_eq!(package, "kubectl");
            assert_eq!(repo, &Some("kubernetes.repo".to_string()));
        } else {
            panic!("Expected extended config");
        }
    }

    #[test]
    fn test_cargo_config() {
        let toml = r#"
        [packages.l]
        tags = ["custom"]
        [packages.l.install]
        cargo = { git = "ssh://git@github.com/user/l.git" }
        "#;

        let config: HashMap<String, HashMap<String, Package>> = toml::from_str(toml).unwrap();
        let pkg = config.get("packages").unwrap().get("l").unwrap();

        if let Some(CargoConfig::Extended { git, .. }) = &pkg.install.cargo {
            assert_eq!(git, &Some("ssh://git@github.com/user/l.git".to_string()));
        } else {
            panic!("Expected extended cargo config");
        }
    }

    #[test]
    fn test_binary_github_config() {
        let toml = r#"
        [packages.obsidian]
        tags = ["desktop"]
        [packages.obsidian.install.binary]
        github = "obsidianmd/obsidian-releases"
        asset_pattern = "Obsidian-{{version}}.AppImage"
        target = "~/.local/share/applications/Obsidian.AppImage"
        "#;

        let config: HashMap<String, HashMap<String, Package>> = toml::from_str(toml).unwrap();
        let pkg = config.get("packages").unwrap().get("obsidian").unwrap();

        if let Some(BinaryConfig::GitHub {
            github,
            asset_pattern,
            target,
            ..
        }) = &pkg.install.binary
        {
            assert_eq!(github, "obsidianmd/obsidian-releases");
            assert_eq!(asset_pattern, "Obsidian-{{version}}.AppImage");
            assert!(target.is_some());
        } else {
            panic!("Expected GitHub binary config");
        }
    }

    #[test]
    fn test_tag_matching() {
        let pkg = Package {
            tags: vec!["cli".to_string(), "dev".to_string()],
            enabled: true,
            ..Default::default()
        };

        // Include matching
        assert!(pkg.matches_tags(&["cli".to_string()], &[]));
        assert!(pkg.matches_tags(&["dev".to_string()], &[]));
        assert!(pkg.matches_tags(&["cli".to_string(), "other".to_string()], &[]));

        // Include not matching
        assert!(!pkg.matches_tags(&["gaming".to_string()], &[]));

        // Exclude matching
        assert!(!pkg.matches_tags(&[], &["cli".to_string()]));
        assert!(!pkg.matches_tags(&["cli".to_string()], &["dev".to_string()]));

        // No filters = include all
        assert!(pkg.matches_tags(&[], &[]));
    }
}
