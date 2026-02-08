//! Line-based patching strategy.
//!
//! Applies regex-based patches to a base file. Patches support:
//! - `set`: Replace lines matching a pattern
//! - `insert`: Add lines after/before a match
//! - `delete`: Remove lines matching a pattern

use super::{DotInstaller, InstallResult};
use crate::config::{resolve_path, Dot, DotFull, LinePatch, LineSetOp, LineInsertOp, LineDeleteOp};
use crate::core::{BombadilError, Result};
use crate::dots::render;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::os::unix;
use std::path::{Path, PathBuf};
use tracing::{debug, instrument, warn};

/// Line-based patch installer.
///
/// This installer:
/// 1. Reads a base file (from system or dotfiles)
/// 2. Loads patch files (TOML format with set/insert/delete operations)
/// 3. Applies patches in order
/// 4. Writes result to .dots and creates symlink
pub struct PatchInstaller;

impl DotInstaller for PatchInstaller {
    #[instrument(skip(self, dot, dotfiles_dir, vars))]
    fn install(
        &self,
        dot: &Dot,
        dotfiles_dir: &Path,
        vars: &tera::Context,
    ) -> Result<InstallResult> {
        let full = match dot {
            Dot::Full(full) => full,
            Dot::Simple { .. } => {
                return Err(BombadilError::ConfigInvalid {
                    message: "Patch strategy requires full dot configuration".to_string(),
                    help: Some("Use strategy = \"patch\" with base and patches fields".to_string()),
                });
            }
        };

        let base_path = full.base.as_ref().ok_or_else(|| BombadilError::ConfigInvalid {
            message: "Patch strategy requires a 'base' field".to_string(),
            help: Some("Add base = \"/path/to/base/file\" to your dot configuration".to_string()),
        })?;

        let target = full.target.as_ref().ok_or_else(|| BombadilError::ConfigInvalid {
            message: "Dot missing target path".to_string(),
            help: Some("Add a 'target' field to the dot configuration".to_string()),
        })?;

        // Resolve paths
        let base_path = resolve_path(base_path);
        let target_path = resolve_path(target);

        // Read base file
        let base_content = if base_path.exists() {
            fs::read_to_string(&base_path).map_err(|e| BombadilError::Io {
                context: format!("reading base file {}", base_path.display()),
                source: e,
            })?
        } else {
            return Err(BombadilError::PatchBaseNotFound {
                path: base_path,
            });
        };

        // Load and apply patches
        let patches = self.load_patches(full, dotfiles_dir)?;

        // Extract variables for template rendering
        let vars_map = extract_vars_from_context(vars);
        let profiles: Vec<String> = vars
            .get("profiles")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(String::from).collect())
            .unwrap_or_default();

        let patched = self.apply_patches(&base_content, &patches, &vars_map, &profiles)?;

        // Write to .dots directory
        let source_name = full.source.as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| target.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "patched".to_string()));

        let dots_dir = dotfiles_dir.join(".dots");
        let copy_path = dots_dir.join(&source_name);

        fs::create_dir_all(copy_path.parent().unwrap_or(&dots_dir)).map_err(|e| BombadilError::Io {
            context: format!("creating .dots directory"),
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

impl PatchInstaller {
    /// Load patch files from the patches directory or source.
    fn load_patches(&self, full: &DotFull, dotfiles_dir: &Path) -> Result<Vec<LinePatch>> {
        let mut patches = Vec::new();

        // Load from patches directory if specified
        if let Some(patches_dir) = &full.patches {
            let patches_path = dotfiles_dir.join(patches_dir);
            if patches_path.is_dir() {
                let mut entries: Vec<_> = fs::read_dir(&patches_path)
                    .map_err(|e| BombadilError::Io {
                        context: format!("reading patches directory {}", patches_path.display()),
                        source: e,
                    })?
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path().extension()
                            .map(|ext| ext == "toml")
                            .unwrap_or(false)
                    })
                    .collect();

                // Sort patches by filename for deterministic ordering
                entries.sort_by_key(|e| e.file_name());

                for entry in entries {
                    let patch = self.load_patch_file(&entry.path())?;
                    patches.push(patch);
                }
            }
        }

        // Load from source if specified (single patch file)
        if let Some(source) = &full.source {
            let source_path = dotfiles_dir.join(source);
            if source_path.exists() && source_path.extension().map(|e| e == "toml").unwrap_or(false) {
                let patch = self.load_patch_file(&source_path)?;
                patches.push(patch);
            }
        }

        Ok(patches)
    }

    /// Load a single patch file.
    fn load_patch_file(&self, path: &Path) -> Result<LinePatch> {
        let content = fs::read_to_string(path).map_err(|e| BombadilError::Io {
            context: format!("reading patch file {}", path.display()),
            source: e,
        })?;

        toml::from_str(&content).map_err(|e| BombadilError::PatchInvalidFormat {
            path: path.to_path_buf(),
            help: format!("TOML parse error: {}", e),
        })
    }

    /// Apply all patches to the base content.
    fn apply_patches(
        &self,
        base: &str,
        patches: &[LinePatch],
        vars: &HashMap<String, String>,
        profiles: &[String],
    ) -> Result<String> {
        let mut lines: Vec<String> = base.lines().map(String::from).collect();

        for patch in patches {
            // Apply set operations
            for op in &patch.set {
                self.apply_set(&mut lines, op, vars, profiles)?;
            }

            // Apply insert operations
            for op in &patch.insert {
                self.apply_insert(&mut lines, op, vars, profiles)?;
            }

            // Apply delete operations
            for op in &patch.delete {
                self.apply_delete(&mut lines, op)?;
            }
        }

        // Rejoin lines with newlines
        let mut result = lines.join("\n");
        if base.ends_with('\n') {
            result.push('\n');
        }

        Ok(result)
    }

    /// Apply a set (replace) operation.
    fn apply_set(
        &self,
        lines: &mut [String],
        op: &LineSetOp,
        vars: &HashMap<String, String>,
        profiles: &[String],
    ) -> Result<()> {
        let regex = Regex::new(&op.pattern).map_err(|e| BombadilError::PatchInvalidFormat {
            path: PathBuf::new(),
            help: format!("Invalid regex pattern '{}': {}", op.pattern, e),
        })?;

        // Render the replacement value as a template
        let rendered_value = render::render_string(
            &op.value,
            Path::new("<patch>"),
            vars,
            &HashMap::new(),
            profiles,
        )?;

        for line in lines.iter_mut() {
            if regex.is_match(line) {
                *line = rendered_value.clone();
                debug!(pattern = %op.pattern, "Applied set operation");
            }
        }

        Ok(())
    }

    /// Apply an insert operation.
    fn apply_insert(
        &self,
        lines: &mut Vec<String>,
        op: &LineInsertOp,
        vars: &HashMap<String, String>,
        profiles: &[String],
    ) -> Result<()> {
        let rendered_lines = render::render_string(
            &op.lines,
            Path::new("<patch>"),
            vars,
            &HashMap::new(),
            profiles,
        )?;

        let new_lines: Vec<String> = rendered_lines.lines().map(String::from).collect();

        if let Some(after_pattern) = &op.after {
            let regex = Regex::new(after_pattern).map_err(|e| BombadilError::PatchInvalidFormat {
                path: PathBuf::new(),
                help: format!("Invalid regex pattern '{}': {}", after_pattern, e),
            })?;

            // Find and insert after matching lines (reverse to preserve indices)
            let matches: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| regex.is_match(l))
                .map(|(i, _)| i)
                .collect();

            for idx in matches.into_iter().rev() {
                for (offset, new_line) in new_lines.iter().enumerate() {
                    lines.insert(idx + 1 + offset, new_line.clone());
                }
                debug!(pattern = %after_pattern, "Applied insert-after operation");
            }
        }

        if let Some(before_pattern) = &op.before {
            let regex = Regex::new(before_pattern).map_err(|e| BombadilError::PatchInvalidFormat {
                path: PathBuf::new(),
                help: format!("Invalid regex pattern '{}': {}", before_pattern, e),
            })?;

            // Find and insert before matching lines (reverse to preserve indices)
            let matches: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| regex.is_match(l))
                .map(|(i, _)| i)
                .collect();

            for idx in matches.into_iter().rev() {
                for (_offset, new_line) in new_lines.iter().rev().enumerate() {
                    lines.insert(idx, new_line.clone());
                }
                debug!(pattern = %before_pattern, "Applied insert-before operation");
            }
        }

        Ok(())
    }

    /// Apply a delete operation.
    fn apply_delete(&self, lines: &mut Vec<String>, op: &LineDeleteOp) -> Result<()> {
        let regex = Regex::new(&op.pattern).map_err(|e| BombadilError::PatchInvalidFormat {
            path: PathBuf::new(),
            help: format!("Invalid regex pattern '{}': {}", op.pattern, e),
        })?;

        lines.retain(|line| {
            let matches = regex.is_match(line);
            if matches {
                debug!(pattern = %op.pattern, "Deleted line");
            }
            !matches
        });

        Ok(())
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

/// Extract variables from a Tera Context into a HashMap.
fn extract_vars_from_context(context: &tera::Context) -> HashMap<String, String> {
    let mut vars = HashMap::new();

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

    fn create_test_patch(set: Vec<LineSetOp>, insert: Vec<LineInsertOp>, delete: Vec<LineDeleteOp>) -> LinePatch {
        LinePatch { set, insert, delete }
    }

    #[test]
    fn apply_set_operation() {
        let installer = PatchInstaller;
        let mut lines = vec![
            "# Comment".to_string(),
            "key = value".to_string(),
            "other = thing".to_string(),
        ];

        let op = LineSetOp {
            pattern: r"^key = .*$".to_string(),
            value: "key = new_value".to_string(),
        };

        installer.apply_set(&mut lines, &op, &HashMap::new(), &[]).unwrap();

        assert_eq!(lines[1], "key = new_value");
    }

    #[test]
    fn apply_insert_after() {
        let installer = PatchInstaller;
        let mut lines = vec![
            "[section]".to_string(),
            "key = value".to_string(),
        ];

        let op = LineInsertOp {
            after: Some(r"^\[section\]$".to_string()),
            before: None,
            lines: "new_key = new_value".to_string(),
        };

        installer.apply_insert(&mut lines, &op, &HashMap::new(), &[]).unwrap();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], "new_key = new_value");
    }

    #[test]
    fn apply_insert_before() {
        let installer = PatchInstaller;
        let mut lines = vec![
            "[section]".to_string(),
            "key = value".to_string(),
        ];

        let op = LineInsertOp {
            after: None,
            before: Some(r"^key = .*$".to_string()),
            lines: "# Comment for key".to_string(),
        };

        installer.apply_insert(&mut lines, &op, &HashMap::new(), &[]).unwrap();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], "# Comment for key");
    }

    #[test]
    fn apply_delete_operation() {
        let installer = PatchInstaller;
        let mut lines = vec![
            "keep_this".to_string(),
            "# delete me".to_string(),
            "keep_this_too".to_string(),
            "# also delete".to_string(),
        ];

        let op = LineDeleteOp {
            pattern: r"^# .*$".to_string(),
        };

        installer.apply_delete(&mut lines, &op).unwrap();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "keep_this");
        assert_eq!(lines[1], "keep_this_too");
    }

    #[test]
    fn apply_patches_with_template_vars() {
        let installer = PatchInstaller;
        let base = "color = #000000\nfont = default";

        let patch = create_test_patch(
            vec![LineSetOp {
                pattern: r"^color = .*$".to_string(),
                value: "color = {{ theme_color }}".to_string(),
            }],
            vec![],
            vec![],
        );

        let mut vars = HashMap::new();
        vars.insert("theme_color".to_string(), "#FF0000".to_string());

        let result = installer.apply_patches(base, &[patch], &vars, &[]).unwrap();

        assert!(result.contains("color = #FF0000"));
        assert!(result.contains("font = default"));
    }

    #[test]
    fn load_and_apply_patch_file() {
        let dir = TempDir::new().unwrap();
        let patch_file = dir.path().join("test.toml");

        fs::write(&patch_file, r#"
            [[set]]
            match = "^old_value$"
            value = "new_value"

            [[insert]]
            after = "^start$"
            lines = "inserted line"

            [[delete]]
            match = "^remove_me$"
        "#).unwrap();

        let installer = PatchInstaller;
        let patch = installer.load_patch_file(&patch_file).unwrap();

        assert_eq!(patch.set.len(), 1);
        assert_eq!(patch.insert.len(), 1);
        assert_eq!(patch.delete.len(), 1);
    }
}
