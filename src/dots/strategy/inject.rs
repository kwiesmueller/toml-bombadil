//! Inject strategy (append/prepend with markers).
//!
//! Adds content to existing files with markers for idempotent updates.
//! This is useful for files like .bashrc or .zshrc where you want to
//! add your customizations without replacing the entire file.

use super::{DotInstaller, InstallResult};
use crate::config::{resolve_path, Dot};
use crate::core::{BombadilError, Result};
use crate::dots::render;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, instrument};

/// Inject installer for append/prepend operations.
///
/// This installer:
/// 1. Reads the existing target file (or creates empty)
/// 2. Removes any existing managed sections (identified by markers)
/// 3. Renders prepend/append content from templates
/// 4. Inserts content with markers for future idempotent updates
/// 5. Writes result (no symlink - modifies file in place or creates copy)
pub struct InjectInstaller;

impl DotInstaller for InjectInstaller {
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
                    message: "Inject strategy requires full dot configuration".to_string(),
                    help: Some("Use strategy = \"inject\" with prepend/append fields".to_string()),
                });
            }
        };

        let target = full.target.as_ref().ok_or_else(|| BombadilError::ConfigInvalid {
            message: "Dot missing target path".to_string(),
            help: Some("Add a 'target' field to the dot configuration".to_string()),
        })?;

        let target_path = resolve_path(target);

        // Get custom marker or use default
        let marker = full.marker.as_deref().unwrap_or("BOMBADIL");
        let marker_start = format!("### {} MANAGED START ###", marker);
        let marker_end = format!("### {} MANAGED END ###", marker);

        // Read existing content
        let existing_content = if target_path.exists() {
            fs::read_to_string(&target_path).map_err(|e| BombadilError::Io {
                context: format!("reading target file {}", target_path.display()),
                source: e,
            })?
        } else {
            String::new()
        };

        // Remove existing managed sections
        let cleaned = remove_managed_sections(&existing_content, &marker_start, &marker_end);

        // Extract variables for template rendering
        let vars_map = extract_vars_from_context(vars);
        let profiles: Vec<String> = vars
            .get("profiles")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(String::from).collect())
            .unwrap_or_default();

        // Build new content
        let mut new_content = String::new();

        // Add prepend content
        if let Some(prepend_path) = &full.prepend {
            let prepend_source = dotfiles_dir.join(prepend_path);
            if prepend_source.exists() {
                let prepend_content = render::render_file(
                    &prepend_source,
                    &vars_map,
                    &HashMap::new(),
                    &profiles,
                )?;

                new_content.push_str(&marker_start);
                new_content.push('\n');
                new_content.push_str(&prepend_content);
                if !prepend_content.ends_with('\n') {
                    new_content.push('\n');
                }
                new_content.push_str(&marker_end);
                new_content.push_str("\n\n");
            }
        }

        // Add original content (with managed sections removed)
        new_content.push_str(&cleaned);

        // Add append content
        if let Some(append_path) = &full.append {
            let append_source = dotfiles_dir.join(append_path);
            if append_source.exists() {
                let append_content = render::render_file(
                    &append_source,
                    &vars_map,
                    &HashMap::new(),
                    &profiles,
                )?;

                if !new_content.ends_with('\n') {
                    new_content.push('\n');
                }
                new_content.push('\n');
                new_content.push_str(&marker_start);
                new_content.push('\n');
                new_content.push_str(&append_content);
                if !append_content.ends_with('\n') {
                    new_content.push('\n');
                }
                new_content.push_str(&marker_end);
                new_content.push('\n');
            }
        }

        // Determine result type
        let result = if !target_path.exists() {
            InstallResult::Created
        } else if existing_content == new_content {
            InstallResult::Unchanged
        } else {
            InstallResult::Updated
        };

        // Write result
        if result != InstallResult::Unchanged {
            // Ensure parent directory exists
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                    context: format!("creating parent directory {}", parent.display()),
                    source: e,
                })?;
            }

            fs::write(&target_path, &new_content).map_err(|e| BombadilError::Io {
                context: format!("writing to {}", target_path.display()),
                source: e,
            })?;

            debug!(
                target = %target_path.display(),
                "Injected content with markers"
            );
        }

        Ok(result)
    }
}

/// Remove managed sections from content (everything between markers, inclusive).
fn remove_managed_sections(content: &str, marker_start: &str, marker_end: &str) -> String {
    let mut result = String::new();
    let mut in_managed_section = false;
    let mut skip_next_empty = false;

    for line in content.lines() {
        if line.contains(marker_start) {
            in_managed_section = true;
            continue;
        }

        if line.contains(marker_end) {
            in_managed_section = false;
            skip_next_empty = true;
            continue;
        }

        if in_managed_section {
            continue;
        }

        // Skip empty lines immediately after managed section end
        if skip_next_empty && line.trim().is_empty() {
            skip_next_empty = false;
            continue;
        }
        skip_next_empty = false;

        result.push_str(line);
        result.push('\n');
    }

    // Remove trailing newlines
    while result.ends_with("\n\n") {
        result.pop();
    }

    result
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
    use crate::config::DotFull;
    use tempfile::TempDir;

    #[test]
    fn remove_managed_sections_single() {
        let content = r#"# User config
export PATH="/usr/bin"

### BOMBADIL MANAGED START ###
export CUSTOM_VAR="value"
### BOMBADIL MANAGED END ###

# More user config
alias ls="ls -la"
"#;

        let result = remove_managed_sections(
            content,
            "### BOMBADIL MANAGED START ###",
            "### BOMBADIL MANAGED END ###",
        );

        assert!(!result.contains("BOMBADIL MANAGED"));
        assert!(!result.contains("CUSTOM_VAR"));
        assert!(result.contains("export PATH"));
        assert!(result.contains("alias ls"));
    }

    #[test]
    fn remove_managed_sections_multiple() {
        let content = r#"### BOMBADIL MANAGED START ###
prepend content
### BOMBADIL MANAGED END ###

user content

### BOMBADIL MANAGED START ###
append content
### BOMBADIL MANAGED END ###
"#;

        let result = remove_managed_sections(
            content,
            "### BOMBADIL MANAGED START ###",
            "### BOMBADIL MANAGED END ###",
        );

        assert!(!result.contains("prepend content"));
        assert!(!result.contains("append content"));
        assert!(result.contains("user content"));
    }

    #[test]
    fn remove_managed_sections_none() {
        let content = "plain content\nno markers\n";

        let result = remove_managed_sections(
            content,
            "### BOMBADIL MANAGED START ###",
            "### BOMBADIL MANAGED END ###",
        );

        assert_eq!(result, "plain content\nno markers\n");
    }

    #[test]
    fn custom_marker() {
        let content = r#"### CUSTOM MANAGED START ###
managed content
### CUSTOM MANAGED END ###

user content
"#;

        let result = remove_managed_sections(
            content,
            "### CUSTOM MANAGED START ###",
            "### CUSTOM MANAGED END ###",
        );

        assert!(!result.contains("managed content"));
        assert!(result.contains("user content"));
    }

    #[test]
    fn inject_creates_new_file() {
        let dir = TempDir::new().unwrap();
        let append_file = dir.path().join("append.sh");
        let target_file = dir.path().join("target.sh");

        fs::write(&append_file, "export MY_VAR=\"{{ value }}\"").unwrap();

        let mut context = tera::Context::new();
        context.insert("value", "test");
        context.insert("profiles", &Vec::<String>::new());

        let full = DotFull {
            source: None,
            target: Some(target_file.clone()),
            strategy: crate::config::DotStrategy::Inject,
            ignore: vec![],
            vars: None,
            base: None,
            patches: None,
            prepend: None,
            append: Some(append_file.file_name().unwrap().into()),
            marker: None,
            hard_copy_target: None,
            hard_copy_permissions: None,
        };

        let installer = InjectInstaller;
        let result = installer
            .install(&Dot::Full(full), dir.path(), &context)
            .unwrap();

        assert_eq!(result, InstallResult::Created);

        let content = fs::read_to_string(&target_file).unwrap();
        assert!(content.contains("BOMBADIL MANAGED START"));
        assert!(content.contains("export MY_VAR=\"test\""));
        assert!(content.contains("BOMBADIL MANAGED END"));
    }

    #[test]
    fn inject_updates_existing_file() {
        let dir = TempDir::new().unwrap();
        let append_file = dir.path().join("append.sh");
        let target_file = dir.path().join("target.sh");

        // Create initial target with user content
        fs::write(&target_file, "# User config\nexport PATH=\"/usr/bin\"\n").unwrap();
        fs::write(&append_file, "export NEW_VAR=\"value\"").unwrap();

        let mut context = tera::Context::new();
        context.insert("profiles", &Vec::<String>::new());

        let full = DotFull {
            source: None,
            target: Some(target_file.clone()),
            strategy: crate::config::DotStrategy::Inject,
            ignore: vec![],
            vars: None,
            base: None,
            patches: None,
            prepend: None,
            append: Some(append_file.file_name().unwrap().into()),
            marker: None,
            hard_copy_target: None,
            hard_copy_permissions: None,
        };

        let installer = InjectInstaller;
        let result = installer
            .install(&Dot::Full(full), dir.path(), &context)
            .unwrap();

        assert_eq!(result, InstallResult::Created); // First time adding managed section

        let content = fs::read_to_string(&target_file).unwrap();
        assert!(content.contains("# User config"));
        assert!(content.contains("export PATH"));
        assert!(content.contains("export NEW_VAR"));
    }

    #[test]
    fn inject_idempotent() {
        let dir = TempDir::new().unwrap();
        let append_file = dir.path().join("append.sh");
        let target_file = dir.path().join("target.sh");

        fs::write(&target_file, "# User config\n").unwrap();
        fs::write(&append_file, "export VAR=\"value\"").unwrap();

        let mut context = tera::Context::new();
        context.insert("profiles", &Vec::<String>::new());

        let full = DotFull {
            source: None,
            target: Some(target_file.clone()),
            strategy: crate::config::DotStrategy::Inject,
            ignore: vec![],
            vars: None,
            base: None,
            patches: None,
            prepend: None,
            append: Some(append_file.file_name().unwrap().into()),
            marker: None,
            hard_copy_target: None,
            hard_copy_permissions: None,
        };

        let installer = InjectInstaller;

        // First install
        installer.install(&Dot::Full(full.clone()), dir.path(), &context).unwrap();
        let first_content = fs::read_to_string(&target_file).unwrap();

        // Second install (should be unchanged)
        let result = installer.install(&Dot::Full(full), dir.path(), &context).unwrap();

        assert_eq!(result, InstallResult::Unchanged);

        let second_content = fs::read_to_string(&target_file).unwrap();
        assert_eq!(first_content, second_content);
    }
}
