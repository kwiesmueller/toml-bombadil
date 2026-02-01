//! Validation module for detecting potential security issues in dotfiles.
//!
//! This module provides functionality to scan dotfiles for:
//! - Unencrypted secrets (passwords, API keys, tokens)
//! - Common sensitive patterns that should be encrypted with GPG
//!
//! # Usage
//!
//! ```bash
//! bombadil validate
//! bombadil validate --strict  # Exit with error on warnings
//! ```

use anyhow::Result;
use colored::*;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Patterns that indicate potential secrets
const SECRET_PATTERNS: &[(&str, &str)] = &[
    (r#"password\s*[:=]\s*["'][^"']+["']"#, "password"),
    (r#"api[_-]?key\s*[:=]\s*["'][^"']+["']"#, "API key"),
    (r#"api[_-]?secret\s*[:=]"#, "API secret"),
    (r#"secret[_-]?key\s*[:=]"#, "secret key"),
    (r#"auth[_-]?token\s*[:=]"#, "auth token"),
    (r#"access[_-]?token\s*[:=]"#, "access token"),
    (r#"private[_-]?key\s*[:=]"#, "private key"),
    (
        r#"BEGIN\s+(RSA|DSA|EC|OPENSSH|PGP)\s+PRIVATE"#,
        "private key block",
    ),
    (r"AKIA[0-9A-Z]{16}", "AWS Access Key ID"),
    (r"ghp_[a-zA-Z0-9]{36}", "GitHub Personal Access Token"),
    (r"gho_[a-zA-Z0-9]{36}", "GitHub OAuth Token"),
    (
        r"github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}",
        "GitHub Fine-grained PAT",
    ),
    (r"sk-[a-zA-Z0-9]{48}", "OpenAI API Key"),
    (r"xox[baprs]-[0-9a-zA-Z-]+", "Slack Token"),
];

/// Files/directories to skip during validation
const SKIP_PATTERNS: &[&str] = &[
    ".git",
    ".dots",
    ".backups",
    "node_modules",
    "target",
    ".claudeignore",
    ".gitignore",
];

/// A warning about a potential secret found in a file
#[derive(Debug)]
pub struct SecretWarning {
    pub file: PathBuf,
    pub line_number: usize,
    pub line_content: String,
    pub pattern_name: String,
}

/// Result of validation
#[derive(Debug, Default)]
pub struct ValidationReport {
    pub warnings: Vec<SecretWarning>,
    pub files_scanned: usize,
    pub secrets_toml_valid: bool,
    pub secrets_toml_issues: Vec<String>,
}

impl ValidationReport {
    pub fn has_issues(&self) -> bool {
        !self.warnings.is_empty() || !self.secrets_toml_valid
    }

    pub fn print(&self) {
        println!("{}", "═".repeat(60).cyan());
        println!("{}", "Bombadil Validation Report".cyan().bold());
        println!("{}", "═".repeat(60).cyan());
        println!();

        println!("Files scanned: {}", self.files_scanned);
        println!();

        // Secrets.toml validation
        if self.secrets_toml_valid {
            println!(
                "{} secrets.toml: All values properly encrypted",
                "✓".green()
            );
        } else {
            println!("{} secrets.toml issues:", "✗".red());
            for issue in &self.secrets_toml_issues {
                println!("  - {}", issue.red());
            }
        }
        println!();

        // Secret warnings
        if self.warnings.is_empty() {
            println!("{} No potential secrets found in dotfiles", "✓".green());
        } else {
            println!(
                "{} Found {} potential secret(s):",
                "⚠".yellow(),
                self.warnings.len()
            );
            println!();

            for warning in &self.warnings {
                println!(
                    "  {} {}:{}",
                    "→".yellow(),
                    warning.file.display(),
                    warning.line_number
                );
                println!("    Pattern: {}", warning.pattern_name.yellow());
                // Truncate long lines
                let content = if warning.line_content.len() > 60 {
                    format!("{}...", &warning.line_content[..60])
                } else {
                    warning.line_content.clone()
                };
                println!("    Content: {}", content.dimmed());
                println!();
            }
        }

        println!("{}", "─".repeat(60));
        if self.has_issues() {
            println!(
                "{} Validation found issues. Consider:",
                "Summary:".yellow().bold()
            );
            println!(
                "  - Encrypting secrets with: bombadil add-secret -k KEY -f secrets.toml --ask"
            );
            println!("  - Adding sensitive files to .gitignore");
            println!("  - Using environment variables for secrets");
        } else {
            println!("{} All checks passed!", "Summary:".green().bold());
        }
    }
}

/// Scan a directory for potential secrets
pub fn scan_directory(dir: &Path) -> Result<ValidationReport> {
    let mut report = ValidationReport::default();

    // Compile regex patterns
    let patterns: Vec<(Regex, &str)> = SECRET_PATTERNS
        .iter()
        .filter_map(|(pattern, name)| {
            Regex::new(&format!("(?i){}", pattern))
                .ok()
                .map(|re| (re, *name))
        })
        .collect();

    // Walk directory
    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| !should_skip(e.path()))
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Skip binary files (simple heuristic)
        if is_likely_binary(path) {
            continue;
        }

        // Special handling for secrets.toml
        if path
            .file_name()
            .map(|n| n == "secrets.toml")
            .unwrap_or(false)
        {
            validate_secrets_toml(path, &mut report);
            report.files_scanned += 1;
            continue;
        }

        // Scan file for secrets
        if let Ok(content) = fs::read_to_string(path) {
            report.files_scanned += 1;

            for (line_num, line) in content.lines().enumerate() {
                // Skip comments
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.starts_with("//") {
                    continue;
                }

                // Check each pattern
                for (regex, name) in &patterns {
                    if regex.is_match(line) {
                        report.warnings.push(SecretWarning {
                            file: path.to_path_buf(),
                            line_number: line_num + 1,
                            line_content: line.trim().to_string(),
                            pattern_name: name.to_string(),
                        });
                        break; // Only report first match per line
                    }
                }
            }
        }
    }

    Ok(report)
}

/// Check if a path should be skipped
fn should_skip(path: &Path) -> bool {
    for pattern in SKIP_PATTERNS {
        if path.components().any(|c| c.as_os_str() == *pattern) {
            return true;
        }
    }
    false
}

/// Check if a file is likely binary
fn is_likely_binary(path: &Path) -> bool {
    let binary_extensions = [
        "png", "jpg", "jpeg", "gif", "ico", "webp", "svg", "woff", "woff2", "ttf", "otf", "eot",
        "zip", "tar", "gz", "bz2", "xz", "exe", "dll", "so", "dylib", "pdf", "doc", "docx",
        "AppImage",
    ];

    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        return binary_extensions.contains(&ext.as_str());
    }

    false
}

/// Validate secrets.toml - ensure all values are GPG encrypted
fn validate_secrets_toml(path: &Path, report: &mut ValidationReport) {
    report.secrets_toml_valid = true;

    if let Ok(content) = fs::read_to_string(path) {
        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Check for key = value pattern
            if let Some((key, value)) = trimmed.split_once('=') {
                let value = value.trim().trim_matches('"').trim_matches('\'');

                // Value should start with gpg: if it contains actual content
                if !value.is_empty() && !value.starts_with("gpg:") {
                    report.secrets_toml_valid = false;
                    report.secrets_toml_issues.push(format!(
                        "Line {}: '{}' has unencrypted value",
                        line_num + 1,
                        key.trim()
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_detects_password_pattern() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "password = \"secret123\"").unwrap();

        let report = scan_directory(dir.path()).unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].pattern_name, "password");
    }

    #[test]
    fn test_detects_aws_key() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "aws_key = AKIAIOSFODNN7EXAMPLE").unwrap();

        let report = scan_directory(dir.path()).unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].pattern_name, "AWS Access Key ID");
    }

    #[test]
    fn test_skips_comments() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "# password = \"secret123\"").unwrap();

        let report = scan_directory(dir.path()).unwrap();
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_validates_secrets_toml() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("secrets.toml");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "encrypted_key = \"gpg:ABC123\"").unwrap();
        writeln!(file, "plain_key = \"not_encrypted\"").unwrap();

        let report = scan_directory(dir.path()).unwrap();
        assert!(!report.secrets_toml_valid);
        assert_eq!(report.secrets_toml_issues.len(), 1);
    }
}
