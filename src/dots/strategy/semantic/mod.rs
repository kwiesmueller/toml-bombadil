//! Semantic patching for structured formats.
//!
//! Provides strategic merge patches for JSON, YAML, TOML, and INI files.
//!
//! Semantic patching allows you to:
//! - Deep merge objects/tables
//! - Delete specific keys
//! - Manipulate arrays (append, prepend, remove)
//! - Apply RFC 6902 JSON Patch operations

pub mod ini;
pub mod json;
pub mod toml_patch;
pub mod yaml;

use super::{DotInstaller, InstallResult};
use crate::config::{resolve_path, Dot, SemanticFormat, SemanticPatch};
use crate::core::{BombadilError, Result};
use std::fs;
use std::os::unix;
use std::path::Path;
use tracing::{debug, instrument};

/// Semantic patch installer.
///
/// This installer:
/// 1. Reads a base file (from system or dotfiles)
/// 2. Loads patch specification (TOML file defining merge/delete/array ops)
/// 3. Parses base file according to format (JSON/YAML/TOML/INI)
/// 4. Applies semantic operations
/// 5. Serializes back to original format
/// 6. Writes to .dots and creates symlink
pub struct SemanticInstaller {
    pub format: SemanticFormat,
}

impl SemanticInstaller {
    pub fn new(format: SemanticFormat) -> Self {
        Self { format }
    }

    /// Detect format from file extension.
    pub fn detect_format(path: &Path) -> SemanticFormat {
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => SemanticFormat::Json,
            Some("yaml") | Some("yml") => SemanticFormat::Yaml,
            Some("toml") => SemanticFormat::Toml,
            Some("ini") | Some("conf") | Some("desktop") => SemanticFormat::Ini,
            _ => SemanticFormat::Json, // Default to JSON
        }
    }
}

impl DotInstaller for SemanticInstaller {
    #[instrument(skip(self, dot, dotfiles_dir, _vars))]
    fn install(
        &self,
        dot: &Dot,
        dotfiles_dir: &Path,
        _vars: &tera::Context,
    ) -> Result<InstallResult> {
        let target = dot.target.as_ref().ok_or_else(|| BombadilError::ConfigInvalid {
            message: "Dot missing target path".to_string(),
            help: Some("Add a 'target' field to the dot configuration".to_string()),
        })?;

        let target_path = resolve_path(target);

        // Determine format
        let format = if self.format != SemanticFormat::default() {
            self.format.clone()
        } else {
            Self::detect_format(&target_path)
        };

        // Load base file
        let base_path = dot.base.as_ref()
            .map(|p| resolve_path(p))
            .or_else(|| {
                // If no base specified and target exists, use target as base
                if target_path.exists() {
                    Some(target_path.clone())
                } else {
                    None
                }
            });

        let base_content = if let Some(ref base) = base_path {
            if base.exists() {
                fs::read_to_string(base).map_err(|e| BombadilError::Io {
                    context: format!("reading base file {}", base.display()),
                    source: e,
                })?
            } else {
                // No base file - start with empty structure
                match format {
                    SemanticFormat::Json => "{}".to_string(),
                    SemanticFormat::Yaml => "{}".to_string(),
                    SemanticFormat::Toml => "".to_string(),
                    SemanticFormat::Ini => "".to_string(),
                }
            }
        } else {
            match format {
                SemanticFormat::Json => "{}".to_string(),
                SemanticFormat::Yaml => "{}".to_string(),
                SemanticFormat::Toml => "".to_string(),
                SemanticFormat::Ini => "".to_string(),
            }
        };

        // Load semantic patch specification
        let patch = self.load_patch_spec(dot, dotfiles_dir)?;

        // Apply patch based on format
        let patched = match format {
            SemanticFormat::Json => json::apply_patch_to_string(&base_content, &patch)?,
            SemanticFormat::Yaml => yaml::apply_patch(&base_content, &patch)?,
            SemanticFormat::Toml => toml_patch::apply_patch(&base_content, &patch)?,
            SemanticFormat::Ini => ini::apply_patch(&base_content, &patch)?,
        };

        debug!(format = ?format, "Applied semantic patch");

        // Write to .dots directory
        let source_name = dot.source.as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| target.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "patched".to_string()));

        let dots_dir = dotfiles_dir.join(".dots");
        let copy_path = dots_dir.join(&source_name);

        fs::create_dir_all(copy_path.parent().unwrap_or(&dots_dir)).map_err(|e| BombadilError::Io {
            context: "creating .dots directory".to_string(),
            source: e,
        })?;

        // Check if content changed
        let result = if copy_path.exists() {
            let existing = fs::read_to_string(&copy_path).unwrap_or_default();
            if existing == patched {
                InstallResult::Unchanged
            } else {
                InstallResult::Updated
            }
        } else {
            InstallResult::Created
        };

        if result != InstallResult::Unchanged {
            fs::write(&copy_path, &patched).map_err(|e| BombadilError::Io {
                context: format!("writing patched file to {}", copy_path.display()),
                source: e,
            })?;
        }

        // Create symlink
        self.create_symlink(&copy_path, &target_path)?;

        Ok(result)
    }
}

impl SemanticInstaller {
    /// Load the semantic patch specification from config.
    fn load_patch_spec(&self, dot: &Dot, dotfiles_dir: &Path) -> Result<SemanticPatch> {
        // If source is specified, it should be a patch spec file
        if let Some(source) = &dot.source {
            let source_path = dotfiles_dir.join(source);
            if source_path.exists() {
                let content = fs::read_to_string(&source_path).map_err(|e| BombadilError::Io {
                    context: format!("reading patch spec from {}", source_path.display()),
                    source: e,
                })?;

                return toml::from_str(&content).map_err(|e| BombadilError::PatchInvalidFormat {
                    path: source_path,
                    help: format!("TOML parse error: {}", e),
                });
            }
        }

        // Otherwise, construct from inline config (future feature)
        Ok(SemanticPatch::default())
    }

    /// Create a symlink from target to copy_path.
    fn create_symlink(&self, copy_path: &Path, target: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                context: format!("creating parent directory {}", parent.display()),
                source: e,
            })?;
        }

        // Remove existing symlink
        if target.is_symlink() {
            fs::remove_file(target).map_err(|e| BombadilError::Io {
                context: format!("removing existing symlink {}", target.display()),
                source: e,
            })?;
        } else if target.exists() {
            // Backup existing file
            let backup = target.with_extension("bombadil-backup");
            fs::rename(target, &backup).map_err(|e| BombadilError::Io {
                context: format!("backing up {} to {}", target.display(), backup.display()),
                source: e,
            })?;
            debug!(
                original = %target.display(),
                backup = %backup.display(),
                "Backed up existing file"
            );
        }

        // Create symlink
        unix::fs::symlink(copy_path, target).map_err(|e| BombadilError::Symlink {
            source: copy_path.to_path_buf(),
            target: target.to_path_buf(),
            cause: e,
        })?;

        debug!(
            source = %copy_path.display(),
            target = %target.display(),
            "Created symlink"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detect_format_from_extension() {
        assert!(matches!(
            SemanticInstaller::detect_format(Path::new("config.json")),
            SemanticFormat::Json
        ));
        assert!(matches!(
            SemanticInstaller::detect_format(Path::new("config.yaml")),
            SemanticFormat::Yaml
        ));
        assert!(matches!(
            SemanticInstaller::detect_format(Path::new("config.yml")),
            SemanticFormat::Yaml
        ));
        assert!(matches!(
            SemanticInstaller::detect_format(Path::new("config.toml")),
            SemanticFormat::Toml
        ));
        assert!(matches!(
            SemanticInstaller::detect_format(Path::new("config.ini")),
            SemanticFormat::Ini
        ));
        assert!(matches!(
            SemanticInstaller::detect_format(Path::new("app.desktop")),
            SemanticFormat::Ini
        ));
    }

    #[test]
    fn semantic_patch_json() {
        let dir = TempDir::new().unwrap();

        // Create base JSON file
        let base_file = dir.path().join("settings.json");
        fs::write(
            &base_file,
            r#"{"theme": "light", "fontSize": 12}"#,
        )
        .unwrap();

        // Create patch spec
        let patch_file = dir.path().join("settings.patch.toml");
        fs::write(
            &patch_file,
            r#"
format = "json"

[merge]
"." = { theme = "dark", newKey = "value" }
"#,
        )
        .unwrap();

        let full = Dot {
            source: Some("settings.patch.toml".into()),
            target: Some(dir.path().join("target.json")),
            strategy: crate::config::DotStrategy::SemanticPatch,
            ignore: vec![],
            vars: None,
            base: Some(base_file.clone()),
            patches: None,
            prepend: None,
            append: None,
            marker: None,
            hard_copy_target: None,
            hard_copy_permissions: None,
        };

        let installer = SemanticInstaller::new(SemanticFormat::Json);
        let result = installer
            .install(&full, dir.path(), &tera::Context::new())
            .unwrap();

        assert!(matches!(result, InstallResult::Created));

        let target_content = fs::read_to_string(dir.path().join("target.json")).unwrap();
        assert!(target_content.contains("dark"));
        assert!(target_content.contains("newKey"));
    }
}
