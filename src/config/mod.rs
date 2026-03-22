//! Configuration types for Bombadil.
//!
//! All types derive `JsonSchema` for editor autocomplete support.
//! Generate the schema with `bombadil schema > bombadil.schema.json`
//!
//! All config loading goes through `LoaderRegistry` to keep the format pluggable.

pub mod loader;
mod schema;
pub mod toml_loader;

pub use loader::{ConfigLoader, LoaderRegistry};
pub use schema::*;

use crate::core::{BombadilError, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{debug, instrument, warn};

/// Load configuration from the XDG config directory with import resolution.
///
/// This is the primary entry point for loading config. It uses `LoaderRegistry`
/// for format detection and recursively resolves all imports.
#[instrument]
pub fn load_config() -> Result<Config> {
    let config_path = config_path()?;
    load_config_resolved(&config_path)
}

/// Load configuration from a specific path via `LoaderRegistry`.
///
/// Does NOT resolve imports. Use `load_config_resolved()` for full resolution.
#[instrument(skip(path))]
pub fn load_config_from(path: &Path) -> Result<Config> {
    let registry = LoaderRegistry::new();
    registry.load(path)
}

/// Load configuration with full import resolution.
///
/// Uses `LoaderRegistry` for format detection, then recursively resolves all
/// `import` paths. Imported files can be any supported format.
#[instrument(skip(path))]
pub fn load_config_resolved(path: &Path) -> Result<Config> {
    let registry = LoaderRegistry::new();
    let mut config = registry.load(path)?;

    let dotfiles_dir = resolve_dotfiles_dir(&config, path);
    let mut visited = HashSet::new();
    visited.insert(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));

    resolve_imports(&mut config, &dotfiles_dir, &mut visited, &registry)?;

    debug!(
        dots = config.settings.dots.len(),
        profiles = config.profiles.len(),
        imports_resolved = visited.len() - 1,
        "configuration loaded with imports resolved"
    );

    Ok(config)
}

/// Recursively resolve imports in a config, preventing cycles.
///
/// Each imported file is loaded via the registry (so imported files can use
/// different formats), merged into `config`, and then its own imports are
/// resolved recursively.
fn resolve_imports(
    config: &mut Config,
    dotfiles_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    registry: &LoaderRegistry,
) -> Result<()> {
    // Take imports out so we can iterate while mutating config
    let imports = std::mem::take(&mut config.import);

    for import_path in &imports {
        let resolved = if import_path.is_absolute() {
            import_path.clone()
        } else {
            dotfiles_dir.join(import_path)
        };

        let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
        if visited.contains(&canonical) {
            warn!(
                path = %canonical.display(),
                "skipping already-visited import (cycle prevention)"
            );
            continue;
        }

        if !resolved.exists() {
            warn!(path = %resolved.display(), "import file not found, skipping");
            continue;
        }

        visited.insert(canonical);

        match registry.load(&resolved) {
            Ok(mut sub_config) => {
                // Recursively resolve the sub-config's own imports
                resolve_imports(&mut sub_config, dotfiles_dir, visited, registry)?;
                merge_config(config, sub_config);
            }
            Err(e) => {
                warn!(
                    path = %resolved.display(),
                    error = %e,
                    "failed to load import, skipping"
                );
            }
        }
    }

    Ok(())
}

/// Merge `source` config into `target`.
///
/// Merge strategy (matches v3 behavior):
/// - dots: add from source, skip if key already exists in target
/// - packages: add from source, skip if key already exists
/// - hooks: append source hooks after target hooks
/// - vars: append source var paths after target var paths
/// - profiles: add from source, skip if name already exists
/// - imports: not merged (already resolved recursively)
fn merge_config(target: &mut Config, source: Config) {
    // Merge dots (source doesn't overwrite existing keys)
    for (key, dot) in source.settings.dots {
        if target.settings.dots.contains_key(&key) {
            debug!(key = %key, "skipping duplicate dot from import");
            continue;
        }
        target.settings.dots.insert(key, dot);
    }

    // Merge packages
    for (key, pkg) in source.settings.packages {
        if target.settings.packages.contains_key(&key) {
            debug!(key = %key, "skipping duplicate package from import");
            continue;
        }
        target.settings.packages.insert(key, pkg);
    }

    // Append hooks
    target.settings.prehooks.extend(source.settings.prehooks);
    target.settings.posthooks.extend(source.settings.posthooks);

    // Append var paths
    target.settings.vars.extend(source.settings.vars);

    // Merge profiles (source doesn't overwrite existing)
    for (key, profile) in source.profiles {
        if target.profiles.contains_key(&key) {
            debug!(key = %key, "skipping duplicate profile from import");
            continue;
        }
        target.profiles.insert(key, profile);
    }
}

/// Resolve the dotfiles directory from config.
///
/// If `config.dotfiles_dir` is set, resolves it (handling ~ and relative paths).
/// Otherwise, uses the parent of the config file path.
pub fn resolve_dotfiles_dir(config: &Config, config_path: &Path) -> PathBuf {
    match &config.dotfiles_dir {
        Some(dir) => {
            let resolved = resolve_path(dir);
            if resolved.is_absolute() {
                resolved
            } else {
                // Relative to $HOME (matching v3 behavior where `dotfiles_dir = "dotfiles"`
                // means `$HOME/dotfiles`)
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(resolved)
            }
        }
        None => {
            // Default: parent directory of the config file
            config_path.parent().unwrap_or(Path::new(".")).to_path_buf()
        }
    }
}

/// Discover all `dots.toml` files under `dotfiles_dir`.
///
/// Returns `(dir_path, dot_file)` pairs where `dir_path` is the directory
/// containing the `dots.toml`, relative to `dotfiles_dir`.
///
/// Discovery rules:
/// - Recursively walk the dotfiles directory.
/// - Any directory containing a `dots.toml` is a managed dot.
/// - A directory is excluded from a parent's file traversal if it has its
///   own `dots.toml` (each `dots.toml` manages only its own directory).
/// - The dotfiles root itself (containing `bombadil.toml`) is excluded
///   unless it also contains a `dots.toml` at the root level.
#[instrument(skip(dotfiles_dir))]
pub fn discover_dot_files(dotfiles_dir: &Path) -> Vec<(PathBuf, DotFile)> {
    let mut results = Vec::new();
    discover_dot_files_recursive(dotfiles_dir, dotfiles_dir, &mut results);
    results
}

fn discover_dot_files_recursive(
    dotfiles_dir: &Path,
    dir: &Path,
    results: &mut Vec<(PathBuf, DotFile)>,
) {
    let dots_toml = dir.join("dots.toml");
    let has_dots_toml = dots_toml.exists();

    if has_dots_toml {
        match std::fs::read_to_string(&dots_toml) {
            Ok(content) => match toml::from_str::<DotFile>(&content) {
                Ok(dot_file) => {
                    let rel = dir.strip_prefix(dotfiles_dir).unwrap_or(dir).to_path_buf();
                    debug!(path = %rel.display(), name = ?dot_file.dot.name, "discovered dots.toml");
                    results.push((rel, dot_file));
                }
                Err(e) => {
                    warn!(path = %dots_toml.display(), error = %e, "failed to parse dots.toml, skipping");
                }
            },
            Err(e) => {
                warn!(path = %dots_toml.display(), error = %e, "failed to read dots.toml, skipping");
            }
        }
    }

    // Recurse into subdirectories regardless of whether this dir has a dots.toml.
    // Each subdirectory with its own dots.toml manages itself independently.
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!(path = %dir.display(), error = %e, "failed to read directory");
            return;
        }
    };

    let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    // Sort for deterministic ordering
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and the .dots directory
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            discover_dot_files_recursive(dotfiles_dir, &path, results);
        }
    }
}

/// Get the path to the bombadil config file in XDG config dir.
pub fn config_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("bombadil.toml"))
        .ok_or_else(|| BombadilError::ConfigInvalid {
            message: "Could not determine XDG_CONFIG_DIR".to_string(),
            help: Some("Ensure $HOME is set".to_string()),
        })
}

/// Resolve a path that may contain ~ or environment variables.
pub fn resolve_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();

    if path_str.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path_str.strip_prefix("~/").unwrap());
        }
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolve_tilde_path() {
        let path = Path::new("~/.config/test");
        let resolved = resolve_path(path);

        if let Some(home) = dirs::home_dir() {
            assert_eq!(resolved, home.join(".config/test"));
        }
    }

    #[test]
    fn resolve_absolute_path() {
        let path = Path::new("/etc/test");
        let resolved = resolve_path(path);
        assert_eq!(resolved, PathBuf::from("/etc/test"));
    }

    #[test]
    fn load_config_with_imports() {
        let dir = TempDir::new().unwrap();

        // Create sub-config with extra dot
        let sub_config_path = dir.path().join("extra.toml");
        fs::write(
            &sub_config_path,
            r#"
            [settings.dots.extra]
            source = "extra/config"
            target = "~/.config/extra"
            "#,
        )
        .unwrap();

        // Create main config that imports sub-config
        let main_config_path = dir.path().join("bombadil.toml");
        fs::write(
            &main_config_path,
            &format!(
                r#"
                import = ["extra.toml"]

                [settings.dots.main]
                source = "main/config"
                target = "~/.config/main"
                "#,
            ),
        )
        .unwrap();

        let config = load_config_resolved(&main_config_path).unwrap();
        assert!(config.settings.dots.contains_key("main"));
        assert!(config.settings.dots.contains_key("extra"));
    }

    #[test]
    fn import_does_not_overwrite_existing_dots() {
        let dir = TempDir::new().unwrap();

        let sub_config_path = dir.path().join("extra.toml");
        fs::write(
            &sub_config_path,
            r#"
            [settings.dots.shared]
            source = "imported/config"
            target = "~/.config/imported"
            "#,
        )
        .unwrap();

        let main_config_path = dir.path().join("bombadil.toml");
        fs::write(
            &main_config_path,
            r#"
            import = ["extra.toml"]

            [settings.dots.shared]
            source = "main/config"
            target = "~/.config/main"
            "#,
        )
        .unwrap();

        let config = load_config_resolved(&main_config_path).unwrap();
        let dot = config.settings.dots.get("shared").unwrap();
        // Main config wins over import
        assert_eq!(dot.source.as_ref().unwrap(), &PathBuf::from("main/config"));
    }

    #[test]
    fn cyclic_imports_handled() {
        let dir = TempDir::new().unwrap();

        let a_path = dir.path().join("a.toml");
        let b_path = dir.path().join("b.toml");

        // a imports b, b imports a
        fs::write(&a_path, r#"import = ["b.toml"]"#).unwrap();
        fs::write(&b_path, r#"import = ["a.toml"]"#).unwrap();

        // Should not infinite loop
        let config = load_config_resolved(&a_path).unwrap();
        assert!(config.settings.dots.is_empty());
    }

    #[test]
    fn merge_hooks_from_imports() {
        let dir = TempDir::new().unwrap();

        let sub_path = dir.path().join("hooks.toml");
        fs::write(
            &sub_path,
            r#"
            [settings]
            prehooks = ["echo imported-pre"]
            posthooks = ["echo imported-post"]
            "#,
        )
        .unwrap();

        let main_path = dir.path().join("bombadil.toml");
        fs::write(
            &main_path,
            r#"
            import = ["hooks.toml"]

            [settings]
            prehooks = ["echo main-pre"]
            posthooks = ["echo main-post"]
            "#,
        )
        .unwrap();

        let config = load_config_resolved(&main_path).unwrap();
        assert_eq!(config.settings.prehooks.len(), 2);
        assert_eq!(config.settings.posthooks.len(), 2);
        // Main hooks come first
        assert_eq!(config.settings.prehooks[0], "echo main-pre");
        assert_eq!(config.settings.prehooks[1], "echo imported-pre");
    }

    #[test]
    fn resolve_dotfiles_dir_from_config() {
        let config = Config {
            dotfiles_dir: Some(PathBuf::from("~/dotfiles")),
            ..Default::default()
        };
        let config_path = Path::new("/home/user/.config/bombadil.toml");
        let resolved = resolve_dotfiles_dir(&config, config_path);

        if let Some(home) = dirs::home_dir() {
            assert_eq!(resolved, home.join("dotfiles"));
        }
    }

    #[test]
    fn resolve_dotfiles_dir_defaults_to_config_parent() {
        let config = Config::default();
        let config_path = Path::new("/home/user/dotfiles/bombadil.toml");
        let resolved = resolve_dotfiles_dir(&config, config_path);
        assert_eq!(resolved, PathBuf::from("/home/user/dotfiles"));
    }

    #[test]
    fn discover_dot_files_finds_nested_dots_toml() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // terminal/zsh/dots.toml
        fs::create_dir_all(root.join("terminal/zsh")).unwrap();
        fs::write(
            root.join("terminal/zsh/dots.toml"),
            r#"[dot]
name = "zsh"

[dot.files]
"zshrc" = "~/.zshrc"
"#,
        )
        .unwrap();

        // terminal/zsh/plugins/dots.toml
        fs::create_dir_all(root.join("terminal/zsh/plugins")).unwrap();
        fs::write(
            root.join("terminal/zsh/plugins/dots.toml"),
            r#"[dot]
name = "zsh-plugins"
depends_on = ["zsh"]

[dot.files]
"plugins.zsh" = "~/.config/zsh/plugins.zsh"
"#,
        )
        .unwrap();

        // editor/nvim/dots.toml
        fs::create_dir_all(root.join("editor/nvim")).unwrap();
        fs::write(
            root.join("editor/nvim/dots.toml"),
            r#"[dot]
name = "nvim"

[dot.files]
"init.lua" = "~/.config/nvim/init.lua"
"#,
        )
        .unwrap();

        let dots = discover_dot_files(root);
        assert_eq!(dots.len(), 3);

        let names: Vec<_> = dots
            .iter()
            .filter_map(|(_, df)| df.dot.name.as_deref())
            .collect();
        assert!(names.contains(&"zsh"), "expected zsh in {:?}", names);
        assert!(names.contains(&"zsh-plugins"), "expected zsh-plugins");
        assert!(names.contains(&"nvim"), "expected nvim");
    }

    #[test]
    fn discover_dot_files_skips_hidden_dirs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // .hidden/dots.toml — should be skipped
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/dots.toml"), "[dot]\nname = \"hidden\"\n").unwrap();

        // visible/dots.toml — should be found
        fs::create_dir_all(root.join("visible")).unwrap();
        fs::write(
            root.join("visible/dots.toml"),
            "[dot]\nname = \"visible\"\n",
        )
        .unwrap();

        let dots = discover_dot_files(root);
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0].1.dot.name.as_deref(), Some("visible"));
    }
}
