//! Configuration schema types with JSON Schema derivation.
//!
//! These types define the structure of `bombadil.toml` and `dots.toml` files.
//!
//! # Two config surfaces
//!
//! - **`bombadil.toml`** → [`Config`]: root config; declares global settings, profiles, vars.
//! - **`dots.toml`** → [`DotFile`]: per-dot config auto-discovered under the dotfiles root.
//!   Contains file mappings, packages, hooks for one named logical unit.

use indexmap::IndexMap;
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

// ─────────────────────────────────────────────────────────────────────────────
// dots.toml types (new file-map model)
// ─────────────────────────────────────────────────────────────────────────────

/// Top-level structure of a `dots.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DotFile {
    pub dot: DotDefinition,
}

/// A named dot: a logical grouping of file mappings, packages, and hooks.
///
/// One `dots.toml` = one named dot. The dot's `name` is the stable identifier
/// used in `depends_on` references across the dotfiles tree.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DotDefinition {
    /// Stable logical name (defaults to directory name). Used in `depends_on`.
    #[serde(default)]
    pub name: Option<String>,

    /// File mappings: source (relative to dots.toml dir) → target path.
    /// Target may use `~/` for home expansion.
    #[serde(default)]
    #[schemars(with = "HashMap<String, FileTarget>")]
    pub files: IndexMap<String, FileTarget>,

    /// Var files for Tera template substitution (relative to dots.toml dir).
    #[serde(default)]
    pub vars: Vec<PathBuf>,

    /// Packages installed before this dot's files are applied.
    #[serde(default)]
    #[schemars(with = "HashMap<String, DotPackage>")]
    pub packages: IndexMap<String, DotPackage>,

    /// Dot names that must be fully applied before this one.
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Tags for conditional inclusion. Empty = always included.
    /// Dot is included only if at least one tag is in the active tag set.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Commands run before this dot is applied (packages and files).
    #[serde(default)]
    pub prehooks: Vec<String>,

    /// Commands run after this dot is fully applied.
    #[serde(default)]
    pub posthooks: Vec<String>,

    /// Per-profile overrides (vars, additional files, tags).
    #[serde(default)]
    #[schemars(with = "HashMap<String, DotProfileOverride>")]
    pub profiles: IndexMap<String, DotProfileOverride>,
}

/// File target: either a plain path string or extended options table.
///
/// `#[serde(untagged)]` works here because TOML distinguishes string and
/// inline-table values at the type level.
///
/// Examples:
/// ```toml
/// "zshrc"    = "~/.zshrc"                          # Simple
/// "config/"  = { target = "~/.config/zsh/", ignore = ["*.bak"] }  # Extended
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FileTarget {
    Simple(String),
    Extended(FileTargetOptions),
}

impl FileTarget {
    pub fn target_path(&self) -> &str {
        match self {
            FileTarget::Simple(s) => s.as_str(),
            FileTarget::Extended(o) => o.target.as_str(),
        }
    }

    pub fn ignore_patterns(&self) -> &[String] {
        match self {
            FileTarget::Simple(_) => &[],
            FileTarget::Extended(o) => &o.ignore,
        }
    }

    pub fn is_copy(&self) -> bool {
        match self {
            FileTarget::Simple(_) => false,
            FileTarget::Extended(o) => o.copy,
        }
    }

    pub fn hard_copy_target(&self) -> Option<&str> {
        match self {
            FileTarget::Simple(_) => None,
            FileTarget::Extended(o) => o.hard_copy_target.as_deref(),
        }
    }

    pub fn hard_copy_permissions(&self) -> Option<u32> {
        match self {
            FileTarget::Simple(_) => None,
            FileTarget::Extended(o) => o.hard_copy_permissions,
        }
    }
}

/// Extended file target options.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileTargetOptions {
    /// Target path (supports `~/` expansion).
    pub target: String,

    /// Glob patterns to exclude from directory copies.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// Copy as a regular file instead of symlinking.
    #[serde(default)]
    pub copy: bool,

    /// Additional hard-copy destination (besides the symlink at `target`).
    ///
    /// Creates a real file at this path after symlinking. Useful when the
    /// consuming program does not follow symlinks (e.g. SDDM session files).
    #[serde(default)]
    pub hard_copy_target: Option<String>,

    /// Unix permissions for the hard copy in octal (e.g. `0o644`).
    #[serde(default)]
    pub hard_copy_permissions: Option<u32>,
}

/// A package declared inside a dots.toml.
///
/// The map key is the canonical cross-platform name. Manager-specific
/// overrides are optional; when absent the key name is used as the package
/// name for the active manager.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DotPackage {
    /// Manager-specific install names. Optional — key name is the fallback.
    #[serde(default)]
    pub install: Option<InstallMethods>,

    /// Tags for conditional inclusion.
    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub prehooks: Vec<String>,

    #[serde(default)]
    pub posthooks: Vec<String>,
}

/// Profile-level overrides within a dots.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DotProfileOverride {
    /// Additional or replacement var files for this profile.
    #[serde(default)]
    pub vars: Vec<PathBuf>,

    /// Additional or replacement file mappings for this profile.
    #[serde(default)]
    #[schemars(with = "HashMap<String, FileTarget>")]
    pub files: IndexMap<String, FileTarget>,

    #[serde(default)]
    pub prehooks: Vec<String>,

    #[serde(default)]
    pub posthooks: Vec<String>,
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

/// Installation method for a system package manager.
///
/// Either a simple package name string or an extended table with optional
/// repository setup (repo file, URL, GPG key) that must be installed first.
///
/// ```toml
/// # Simple
/// dnf = "ripgrep"
///
/// # Extended — installs a repo file before the package
/// dnf = { package = "kubectl", repo = "k8s/kubectl.repo" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PkgManagerInstall {
    /// Simple package name.
    Simple(String),
    /// Extended configuration with optional repository setup.
    Extended {
        /// Package name to install.
        package: String,
        /// Repo file path relative to the dotfiles root to copy into place
        /// before installing (e.g. `"k8s/kubectl.repo"` → `/etc/yum.repos.d/kubectl.repo`).
        #[serde(default)]
        repo: Option<String>,
        /// Repository URL to add via the manager's repo command.
        #[serde(default)]
        repo_url: Option<String>,
        /// GPG key URL to import before adding the repository.
        #[serde(default)]
        gpg_key: Option<String>,
    },
}

impl PkgManagerInstall {
    /// The package name to pass to the package manager install command.
    pub fn package_name(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Extended { package, .. } => package.as_str(),
        }
    }

    /// Path (relative to dotfiles root) of a repo file to install before this package.
    pub fn repo_file(&self) -> Option<&str> {
        match self {
            Self::Simple(_) => None,
            Self::Extended { repo, .. } => repo.as_deref(),
        }
    }
}

/// Installation methods for different package managers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct InstallMethods {
    /// DNF package (Fedora/RHEL) — simple name or extended with repo setup.
    #[serde(default)]
    pub dnf: Option<PkgManagerInstall>,

    /// APT package (Debian/Ubuntu) — simple name or extended with repo setup.
    #[serde(default)]
    pub apt: Option<PkgManagerInstall>,

    /// Homebrew package (macOS/Linux) — simple name or extended with repo setup.
    #[serde(default)]
    pub brew: Option<PkgManagerInstall>,

    /// Pacman package (Arch) — simple name or extended with repo setup.
    #[serde(default)]
    pub pacman: Option<PkgManagerInstall>,

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
///
/// Profiles are the primary mechanism for machine/environment differences.
/// One profile per device is committed to the repo; the active profile is
/// stored in `.active_profile` (gitignored) written by `bombadil init`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    /// Profiles this profile inherits from (last-wins merge on vars + active_tags + hooks).
    #[serde(default)]
    pub inherits: Vec<String>,

    /// Extra profiles to enable (legacy alias for `inherits`).
    #[serde(default)]
    pub extra_profiles: Vec<String>,

    /// Tags explicitly active for this profile.
    ///
    /// These supplement auto-detected platform tags (os, distro, desktop env).
    /// Useful when setting up a fresh system where the desktop env is not yet
    /// installed and thus not auto-detectable.
    #[serde(default)]
    pub active_tags: Vec<String>,

    /// Tags for which to exclude packages/dots even if auto-detected.
    #[serde(default)]
    pub package_exclude_tags: Vec<String>,

    /// Dotfile overrides for this profile (legacy bombadil.toml style).
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

    /// Package tags to enable for this profile (legacy, prefer `active_tags`).
    #[serde(default)]
    pub package_tags: Vec<String>,
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

/// Generate JSON Schema for the root configuration (bombadil.toml).
pub fn generate_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(Config)
}

/// Generate JSON Schema for a dots.toml file.
pub fn generate_dot_file_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(DotFile)
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
        assert_eq!(
            pkg.install.dnf.as_ref().map(|d| d.package_name()),
            Some("ripgrep")
        );
    }

    #[test]
    fn parse_dot_file_simple_files() {
        let toml = r#"
            [dot]
            name = "zsh"

            [dot.files]
            "zshrc"  = "~/.zshrc"
            "zshenv" = "~/.zshenv"
        "#;

        let dot_file: DotFile = toml::from_str(toml).unwrap();
        assert_eq!(dot_file.dot.name.as_deref(), Some("zsh"));
        assert_eq!(dot_file.dot.files.len(), 2);
        assert_eq!(
            dot_file.dot.files["zshrc"].target_path(),
            "~/.zshrc"
        );
    }

    #[test]
    fn parse_dot_file_extended_target() {
        let toml = r#"
            [dot]
            name = "kitty"

            [dot.files]
            "kitty.conf" = "~/.config/kitty/kitty.conf"
            "themes/" = { target = "~/.config/kitty/themes/", ignore = ["*.bak"] }
        "#;

        let dot_file: DotFile = toml::from_str(toml).unwrap();
        let themes = &dot_file.dot.files["themes/"];
        assert_eq!(themes.target_path(), "~/.config/kitty/themes/");
        assert_eq!(themes.ignore_patterns(), &["*.bak"]);
    }

    #[test]
    fn parse_dot_file_with_packages() {
        let toml = r#"
            [dot]
            name = "nvim"
            prehooks = ["mkdir -p ~/.local/share/nvim"]

            [dot.files]
            "init.lua" = "~/.config/nvim/init.lua"

            [dot.packages.neovim]
            install.dnf  = "neovim"
            install.brew = "neovim"
            posthooks = ["nvim --headless '+checkhealth' +qa"]
        "#;

        let dot_file: DotFile = toml::from_str(toml).unwrap();
        assert_eq!(dot_file.dot.prehooks, vec!["mkdir -p ~/.local/share/nvim"]);
        let nvim_pkg = &dot_file.dot.packages["neovim"];
        assert_eq!(
            nvim_pkg.install.as_ref().unwrap().dnf.as_ref().map(|d| d.package_name()),
            Some("neovim")
        );
        assert_eq!(nvim_pkg.posthooks.len(), 1);
    }

    #[test]
    fn parse_dot_file_with_profile_override() {
        let toml = r#"
            [dot]
            name = "kitty"

            [dot.files]
            "kitty.conf" = "~/.config/kitty/kitty.conf"

            [dot.profiles.work]
            vars = ["profiles/work.toml"]
        "#;

        let dot_file: DotFile = toml::from_str(toml).unwrap();
        let work = &dot_file.dot.profiles["work"];
        assert_eq!(work.vars.len(), 1);
    }

    #[test]
    fn parse_profile_active_tags() {
        let toml = r#"
            [profiles.fedora-kde]
            active_tags = ["gui", "kde", "wayland", "fedora"]
            vars = ["systems/fedora.toml"]
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let profile = &config.profiles["fedora-kde"];
        assert_eq!(profile.active_tags, vec!["gui", "kde", "wayland", "fedora"]);
    }
}
