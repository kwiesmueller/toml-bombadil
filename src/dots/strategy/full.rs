//! Full file replacement strategy (symlinks).
//!
//! This is the default strategy - renders templates and creates symlinks.

use super::{DotInstaller, InstallResult};
use crate::config::{resolve_path, Dot};
use crate::core::{BombadilError, Result};
use crate::dots::render;
use std::collections::HashMap;
use std::fs;
use std::os::unix;
use std::path::{Path, PathBuf};
use tracing::{debug, instrument, warn};

/// Full file replacement installer.
///
/// This installer:
/// 1. Renders templates with variable substitution
/// 2. Copies rendered files to a `.dots` directory
/// 3. Creates symlinks from target to the rendered copies
pub struct FullInstaller;

impl DotInstaller for FullInstaller {
    #[instrument(skip(self, dot, dotfiles_dir, vars))]
    fn install(
        &self,
        dot: &Dot,
        dotfiles_dir: &Path,
        vars: &tera::Context,
    ) -> Result<InstallResult> {
        let source = dot
            .source
            .clone()
            .ok_or_else(|| BombadilError::ConfigInvalid {
                message: "Dot missing source path".to_string(),
                help: Some("Add a 'source' field to the dot configuration".to_string()),
            })?;
        let target = dot
            .target
            .clone()
            .ok_or_else(|| BombadilError::ConfigInvalid {
                message: "Dot missing target path".to_string(),
                help: Some("Add a 'target' field to the dot configuration".to_string()),
            })?;
        let ignore = dot.ignore.clone();

        let source_path = dotfiles_dir.join(&source);
        let target_path = resolve_path(&target);
        let dots_dir = dotfiles_dir.join(".dots");

        // Extract variables from Tera context
        let vars_map = extract_vars_from_context(vars);
        let secrets_map = HashMap::new(); // TODO: integrate with GPG secrets

        // Get profiles from context
        let profiles: Vec<String> = vars
            .get("profiles")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        // Build ignored paths
        let ignored = build_ignored_paths(&source_path, &ignore)?;

        // Process the source (file or directory)
        let result = self.process_source(
            &source_path,
            &dots_dir.join(&source),
            &target_path,
            &ignored,
            &vars_map,
            &secrets_map,
            &profiles,
        )?;

        Ok(result)
    }
}

impl FullInstaller {
    /// Process a source path (file or directory) recursively.
    #[allow(clippy::too_many_arguments)]
    fn process_source(
        &self,
        source: &Path,
        copy_path: &Path,
        target: &Path,
        ignored: &[PathBuf],
        vars: &HashMap<String, String>,
        secrets: &HashMap<String, String>,
        profiles: &[String],
    ) -> Result<InstallResult> {
        if ignored.contains(&source.to_path_buf()) {
            debug!(path = %source.display(), "Ignoring path");
            return Ok(InstallResult::Ignored);
        }

        if source.is_file() {
            self.process_file(source, copy_path, target, vars, secrets, profiles)
        } else if source.is_dir() {
            self.process_directory(source, copy_path, target, ignored, vars, secrets, profiles)
        } else {
            warn!(path = %source.display(), "Source is neither file nor directory");
            Ok(InstallResult::Ignored)
        }
    }

    /// Process a single file: render template, write to .dots, create symlink.
    fn process_file(
        &self,
        source: &Path,
        copy_path: &Path,
        target: &Path,
        vars: &HashMap<String, String>,
        secrets: &HashMap<String, String>,
        profiles: &[String],
    ) -> Result<InstallResult> {
        // Ensure parent directories exist
        if let Some(parent) = copy_path.parent() {
            fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                context: format!("creating directory {}", parent.display()),
                source: e,
            })?;
        }

        // Try to render as template, fall back to raw copy
        let content = match render::render_file(source, vars, secrets, profiles) {
            Ok(rendered) => rendered.into_bytes(),
            Err(_) => {
                // Non-UTF8 or template error - copy raw
                debug!(path = %source.display(), "Copying raw (non-template) file");
                fs::read(source).map_err(|e| BombadilError::Io {
                    context: format!("reading source file {}", source.display()),
                    source: e,
                })?
            }
        };

        // Check if content changed
        let result = if copy_path.exists() {
            let existing = fs::read(copy_path).unwrap_or_default();
            if existing == content {
                InstallResult::Unchanged
            } else {
                InstallResult::Updated
            }
        } else {
            InstallResult::Created
        };

        // Write to .dots directory
        if result != InstallResult::Unchanged {
            fs::write(copy_path, &content).map_err(|e| BombadilError::Io {
                context: format!("writing to {}", copy_path.display()),
                source: e,
            })?;

            // Preserve permissions
            if let Ok(metadata) = source.metadata() {
                let _ = fs::set_permissions(copy_path, metadata.permissions());
            }
        }

        // Create symlink
        self.create_symlink(copy_path, target)?;

        Ok(result)
    }

    /// Process a directory recursively.
    #[allow(clippy::too_many_arguments)]
    fn process_directory(
        &self,
        source: &Path,
        copy_path: &Path,
        target: &Path,
        ignored: &[PathBuf],
        vars: &HashMap<String, String>,
        secrets: &HashMap<String, String>,
        profiles: &[String],
    ) -> Result<InstallResult> {
        fs::create_dir_all(copy_path).map_err(|e| BombadilError::Io {
            context: format!("creating directory {}", copy_path.display()),
            source: e,
        })?;

        let mut results = Vec::new();

        for entry in fs::read_dir(source).map_err(|e| BombadilError::Io {
            context: format!("reading directory {}", source.display()),
            source: e,
        })? {
            let entry = entry.map_err(|e| BombadilError::Io {
                context: format!("reading directory entry in {}", source.display()),
                source: e,
            })?;

            let entry_name = entry.file_name();
            let entry_name_str = entry_name.to_string_lossy();

            let result = self.process_source(
                &source.join(&*entry_name_str),
                &copy_path.join(&*entry_name_str),
                &target.join(&*entry_name_str),
                ignored,
                vars,
                secrets,
                profiles,
            );

            match result {
                Ok(r) => results.push(r),
                Err(e) => warn!(error = %e, "Error processing entry"),
            }
        }

        // Create symlink for the directory
        self.create_symlink(copy_path, target)?;

        // Aggregate results
        if results.contains(&InstallResult::Updated) {
            Ok(InstallResult::Updated)
        } else if results.contains(&InstallResult::Created) {
            Ok(InstallResult::Created)
        } else {
            Ok(InstallResult::Unchanged)
        }
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

        // Remove existing symlink or file
        if target.is_symlink() || target.exists() {
            if target.is_symlink() {
                fs::remove_file(target).map_err(|e| BombadilError::Io {
                    context: format!("removing existing symlink {}", target.display()),
                    source: e,
                })?;
            } else if target.is_dir() {
                // Don't remove directories - might be system dirs
                debug!(
                    target = %target.display(),
                    "Target is a directory, not creating symlink"
                );
                return Ok(());
            } else {
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

/// Build a list of ignored paths based on glob patterns.
fn build_ignored_paths(source: &Path, patterns: &[String]) -> Result<Vec<PathBuf>> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }

    let source_str = source.to_string_lossy();
    let walker = globwalk::GlobWalkerBuilder::from_patterns(&*source_str, patterns)
        .build()
        .map_err(|e| BombadilError::ConfigInvalid {
            message: format!("Invalid ignore pattern: {}", e),
            help: None,
        })?;

    Ok(walker
        .filter_map(|e| e.ok())
        .map(|e| e.path().to_path_buf())
        .collect())
}

/// Extract variables from a Tera Context into a HashMap.
fn extract_vars_from_context(context: &tera::Context) -> HashMap<String, String> {
    let mut vars = HashMap::new();

    // Tera Context serializes to JSON Value directly
    let json: serde_json::Value = context.clone().into_json();
    if let Some(obj) = json.as_object() {
        for (key, value) in obj {
            if let Some(s) = value.as_str() {
                vars.insert(key.clone(), s.to_string());
            }
        }
    }

    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn process_simple_file() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("source.txt");
        let dots = dir.path().join(".dots");
        let target = dir.path().join("target.txt");

        fs::write(&source, "Hello {{ name }}!").unwrap();
        fs::create_dir_all(&dots).unwrap();

        let installer = FullInstaller;
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Tom".to_string());

        let result = installer
            .process_file(
                &source,
                &dots.join("source.txt"),
                &target,
                &vars,
                &HashMap::new(),
                &[],
            )
            .unwrap();

        assert_eq!(result, InstallResult::Created);
        assert!(target.is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "Hello Tom!");
    }

    #[test]
    fn process_binary_file() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("binary.bin");
        let dots = dir.path().join(".dots");
        let target = dir.path().join("target.bin");

        // Write binary content
        fs::write(&source, &[0x00, 0x01, 0xFF, 0xFE]).unwrap();
        fs::create_dir_all(&dots).unwrap();

        let installer = FullInstaller;
        let result = installer
            .process_file(
                &source,
                &dots.join("binary.bin"),
                &target,
                &HashMap::new(),
                &HashMap::new(),
                &[],
            )
            .unwrap();

        assert_eq!(result, InstallResult::Created);
        assert_eq!(fs::read(&target).unwrap(), vec![0x00, 0x01, 0xFF, 0xFE]);
    }

    #[test]
    fn unchanged_file_not_rewritten() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("source.txt");
        let dots = dir.path().join(".dots");
        let target = dir.path().join("target.txt");

        fs::write(&source, "Hello!").unwrap();
        fs::create_dir_all(&dots).unwrap();

        let installer = FullInstaller;

        // First install
        let result1 = installer
            .process_file(
                &source,
                &dots.join("source.txt"),
                &target,
                &HashMap::new(),
                &HashMap::new(),
                &[],
            )
            .unwrap();
        assert_eq!(result1, InstallResult::Created);

        // Second install (unchanged)
        let result2 = installer
            .process_file(
                &source,
                &dots.join("source.txt"),
                &target,
                &HashMap::new(),
                &HashMap::new(),
                &[],
            )
            .unwrap();
        assert_eq!(result2, InstallResult::Unchanged);
    }
}
