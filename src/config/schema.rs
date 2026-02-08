//! Configuration schema types with JSON Schema derivation.
//!
//! These types define the structure of bombadil.toml and all related config files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Root configuration for Bombadil.
///
/// This is the structure of `bombadil.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// Path to the dotfiles directory (relative to config file or absolute).
    #[serde(default)]
    pub dotfiles_dir: Option<PathBuf>,

    /// GPG user ID for encrypting/decrypting secrets.
    #[serde(default)]
    pub gpg_user_id: Option<String>,

    /// Main settings section.
    #[serde(default)]
    pub settings: Settings,

    /// Named profiles for different environments/machines.
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,

    /// Import additional config files.
    #[serde(default)]
    pub import: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dotfiles_dir: None,
            gpg_user_id: None,
            settings: Settings::default(),
            profiles: HashMap::new(),
            import: Vec::new(),
        }
    }
}

/// Main settings section containing dots, packages, hooks, and variables.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Settings {
    /// Dotfile entries to manage.
    #[serde(default)]
    pub dots: HashMap<String, Dot>,

    /// Package definitions.
    #[serde(default)]
    pub packages: HashMap<String, Package>,

    /// Variable files to load.
    #[serde(default)]
    pub vars: Vec<PathBuf>,

    /// Commands to run before linking.
    #[serde(default)]
    pub prehooks: Vec<String>,

    /// Commands to run after linking.
    #[serde(default)]
    pub posthooks: Vec<String>,

    /// Whether to run hooks in the dotfiles directory.
    #[serde(default)]
    pub run_hooks_in_dotfiles_dir: bool,
}

/// A dotfile entry describing how to manage a config file or directory.
///
/// Simple dots need only `source` and `target`. Strategy-specific fields
/// (base, patches, prepend, append, marker) are used by their respective strategies.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Dot {
    /// Source path relative to dotfiles directory.
    #[serde(default)]
    pub source: Option<PathBuf>,

    /// Target path (supports ~ expansion).
    #[serde(default)]
    pub target: Option<PathBuf>,

    /// Strategy for managing this dotfile.
    #[serde(default)]
    pub strategy: DotStrategy,

    /// Glob patterns to ignore within this dot.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// Local variables file for this dot.
    #[serde(default)]
    pub vars: Option<PathBuf>,

    /// For patch strategy: base file to patch against.
    #[serde(default)]
    pub base: Option<PathBuf>,

    /// For patch strategy: directory containing patch files.
    #[serde(default)]
    pub patches: Option<PathBuf>,

    /// For inject strategy: content to prepend.
    #[serde(default)]
    pub prepend: Option<PathBuf>,

    /// For inject strategy: content to append.
    #[serde(default)]
    pub append: Option<PathBuf>,

    /// For inject strategy: marker for idempotent injection.
    #[serde(default)]
    pub marker: Option<String>,

    /// Path for hard copy (instead of symlink).
    #[serde(default)]
    pub hard_copy_target: Option<PathBuf>,

    /// Permissions for hard copy (octal, e.g., 0o644).
    #[serde(default)]
    pub hard_copy_permissions: Option<u32>,
}

/// Strategy for managing a dotfile.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DotStrategy {
    /// Full file replacement with symlink (default).
    #[default]
    Full,

    /// Line-based patching against a base file.
    Patch,

    /// Semantic patching for structured formats (JSON, YAML, TOML).
    SemanticPatch,

    /// Append/prepend to existing file with markers.
    Inject,
}

/// Package definition with multi-manager support.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Package {
    /// Tags for filtering packages.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Whether this package is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Installation methods by package manager.
    #[serde(default)]
    pub install: InstallMethods,
}

fn default_true() -> bool {
    true
}

/// Installation methods for different package managers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct InstallMethods {
    /// DNF package name (Fedora/RHEL).
    #[serde(default)]
    pub dnf: Option<String>,

    /// APT package name (Debian/Ubuntu).
    #[serde(default)]
    pub apt: Option<String>,

    /// Homebrew package name (macOS/Linux).
    #[serde(default)]
    pub brew: Option<String>,

    /// Pacman package name (Arch).
    #[serde(default)]
    pub pacman: Option<String>,

    /// Cargo crate name.
    #[serde(default)]
    pub cargo: Option<CargoInstall>,

    /// Go module path.
    #[serde(default)]
    pub go: Option<String>,

    /// Binary download configuration.
    #[serde(default)]
    pub binary: Option<BinaryInstall>,

    /// Flatpak application ID.
    #[serde(default)]
    pub flatpak: Option<String>,

    /// Git repository to clone and build.
    #[serde(default)]
    pub git: Option<GitInstall>,
}

/// Cargo installation configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CargoInstall {
    /// Simple crate name.
    Simple(String),

    /// Full configuration with features, git source, etc.
    Full {
        /// Crate name or git URL.
        name: String,

        /// Git repository URL (instead of crates.io).
        #[serde(default)]
        git: Option<String>,

        /// Features to enable.
        #[serde(default)]
        features: Vec<String>,

        /// Use --locked flag.
        #[serde(default)]
        locked: bool,
    },
}

/// Binary download configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BinaryInstall {
    /// Direct URL to download.
    #[serde(default)]
    pub url: Option<String>,

    /// GitHub repository (owner/repo format).
    #[serde(default)]
    pub github: Option<String>,

    /// Asset filename pattern (supports {{version}}, {{arch}}, {{os}}).
    #[serde(default)]
    pub asset_pattern: Option<String>,

    /// Specific version to install.
    #[serde(default)]
    pub version: Option<String>,

    /// Binary name within archive.
    #[serde(default)]
    pub binary_name: Option<String>,

    /// Installation directory.
    #[serde(default)]
    pub install_dir: Option<PathBuf>,
}

/// Git clone and build configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitInstall {
    /// Git repository URL.
    pub url: String,

    /// Branch or tag to checkout.
    #[serde(default)]
    pub ref_: Option<String>,

    /// Build command to run after clone.
    #[serde(default)]
    pub build: Option<String>,

    /// Install command to run after build.
    #[serde(default)]
    pub install: Option<String>,
}

/// Profile for environment-specific configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    /// Profiles this profile inherits from.
    #[serde(default)]
    pub inherits: Vec<String>,

    /// Extra profiles to enable (legacy, prefer `inherits`).
    #[serde(default)]
    pub extra_profiles: Vec<String>,

    /// Dotfile overrides for this profile.
    #[serde(default)]
    pub dots: HashMap<String, DotOverride>,

    /// Additional variable files for this profile.
    #[serde(default)]
    pub vars: Vec<PathBuf>,

    /// Pre-hooks specific to this profile.
    #[serde(default)]
    pub prehooks: Vec<String>,

    /// Post-hooks specific to this profile.
    #[serde(default)]
    pub posthooks: Vec<String>,

    /// Whether to run hooks in dotfiles directory.
    #[serde(default)]
    pub run_hooks_in_dotfiles_dir: bool,

    /// Package tags to enable for this profile.
    #[serde(default)]
    pub package_tags: Vec<String>,

    /// Package tags to disable for this profile.
    #[serde(default)]
    pub package_exclude_tags: Vec<String>,
}

/// Dotfile override in a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DotOverride {
    /// Override source path.
    #[serde(default)]
    pub source: Option<PathBuf>,

    /// Override target path.
    #[serde(default)]
    pub target: Option<PathBuf>,

    /// Override strategy.
    #[serde(default)]
    pub strategy: Option<DotStrategy>,

    /// Override ignore patterns.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// Override local variables file.
    #[serde(default)]
    pub vars: Option<PathBuf>,

    /// Override hard copy target.
    #[serde(default)]
    pub hard_copy_target: Option<PathBuf>,

    /// Override hard copy permissions.
    #[serde(default)]
    pub hard_copy_permissions: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Patch Configuration Types
// ─────────────────────────────────────────────────────────────────────────────

/// Line-based patch file format.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct LinePatch {
    /// Set operations (replace matching lines).
    #[serde(default)]
    pub set: Vec<LineSetOp>,

    /// Insert operations (add lines after/before match).
    #[serde(default)]
    pub insert: Vec<LineInsertOp>,

    /// Delete operations (remove matching lines).
    #[serde(default)]
    pub delete: Vec<LineDeleteOp>,
}

/// Replace a line matching a pattern.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LineSetOp {
    /// Regex pattern to match.
    #[serde(rename = "match")]
    pub pattern: String,

    /// Replacement value.
    pub value: String,
}

/// Insert lines relative to a match.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LineInsertOp {
    /// Insert after line matching this pattern.
    #[serde(default)]
    pub after: Option<String>,

    /// Insert before line matching this pattern.
    #[serde(default)]
    pub before: Option<String>,

    /// Lines to insert.
    pub lines: String,
}

/// Delete lines matching a pattern.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LineDeleteOp {
    /// Regex pattern to match lines to delete.
    #[serde(rename = "match")]
    pub pattern: String,
}

/// Semantic patch file format for structured data.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SemanticPatch {
    /// Format of the target file.
    pub format: SemanticFormat,

    /// Base file to patch (optional).
    #[serde(default)]
    pub base: Option<PathBuf>,

    /// Keys/values to merge.
    #[serde(default)]
    pub merge: HashMap<String, serde_json::Value>,

    /// Keys to delete.
    #[serde(default)]
    pub delete: Vec<String>,

    /// Array operations.
    #[serde(default)]
    pub arrays: HashMap<String, Vec<ArrayOp>>,

    /// RFC 6902 JSON Patch operations (advanced).
    #[serde(default)]
    pub json_patch: Vec<serde_json::Value>,
}

/// Supported semantic patch formats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SemanticFormat {
    #[default]
    Json,
    Yaml,
    Toml,
    Ini,
}

/// Array operation in semantic patches.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArrayOp {
    /// Operation type.
    pub operation: ArrayOpType,

    /// Value(s) for the operation.
    pub value: serde_json::Value,
}

/// Array operation types.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArrayOpType {
    /// Replace entire array.
    Set,
    /// Append to array.
    Append,
    /// Prepend to array.
    Prepend,
    /// Remove matching elements.
    Remove,
}

// ─────────────────────────────────────────────────────────────────────────────
// Schema Generation
// ─────────────────────────────────────────────────────────────────────────────

/// Generate JSON Schema for the configuration.
pub fn generate_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(Config)
}

/// Generate JSON Schema for line patches.
pub fn generate_patch_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(LinePatch)
}

/// Generate JSON Schema for semantic patches.
pub fn generate_semantic_patch_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(SemanticPatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
            [settings.dots.kitty]
            source = "kitty/kitty.conf"
            target = "~/.config/kitty/kitty.conf"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.settings.dots.contains_key("kitty"));
    }

    #[test]
    fn parse_patch_dot() {
        let toml = r#"
            [settings.dots.sway]
            strategy = "patch"
            base = "/etc/sway/config"
            patches = "sway/patches/"
            target = "~/.config/sway/config"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let dot = config.settings.dots.get("sway").unwrap();
        assert!(matches!(dot.strategy, DotStrategy::Patch));
        assert!(dot.base.is_some());
    }

    #[test]
    fn generate_valid_schema() {
        let schema = generate_schema();
        let json = serde_json::to_string_pretty(&schema).unwrap();
        assert!(json.contains("dotfiles_dir"));
        assert!(json.contains("settings"));
        assert!(json.contains("profiles"));
    }

    #[test]
    fn parse_package() {
        let toml = r#"
            [settings.packages.ripgrep]
            tags = ["cli", "essential"]
            install.dnf = "ripgrep"
            install.cargo = "ripgrep"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let pkg = config.settings.packages.get("ripgrep").unwrap();
        assert_eq!(pkg.tags, vec!["cli", "essential"]);
        assert_eq!(pkg.install.dnf, Some("ripgrep".to_string()));
    }
}
