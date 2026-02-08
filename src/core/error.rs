//! Error types for Bombadil.
//!
//! Uses `miette` for rich error reporting with source snippets and help text.

use miette::Diagnostic;
use std::path::PathBuf;
use thiserror::Error;

/// Result type alias using BombadilError.
pub type Result<T> = std::result::Result<T, BombadilError>;

/// Main error type for Bombadil operations.
#[derive(Error, Diagnostic, Debug)]
pub enum BombadilError {
    // ─────────────────────────────────────────────────────────────────────────
    // Configuration Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("Configuration file not found: {path}")]
    #[diagnostic(
        code(bombadil::config::not_found),
        help("Run `bombadil install <path>` to set up your dotfiles directory")
    )]
    ConfigNotFound { path: PathBuf },

    #[error("Failed to parse configuration")]
    #[diagnostic(code(bombadil::config::parse))]
    ConfigParse {
        #[source]
        source: toml::de::Error,
        path: PathBuf,
    },

    #[error("Invalid configuration: {message}")]
    #[diagnostic(code(bombadil::config::invalid))]
    ConfigInvalid {
        message: String,
        #[help]
        help: Option<String>,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Dotfile Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("Dotfile source not found: {path}")]
    #[diagnostic(
        code(bombadil::dots::source_not_found),
        help("Check that the source path exists in your dotfiles directory")
    )]
    SourceNotFound { path: PathBuf },

    #[error("Target directory does not exist: {path}")]
    #[diagnostic(
        code(bombadil::dots::target_not_found),
        help("The parent directory of the target must exist")
    )]
    TargetNotFound { path: PathBuf },

    #[error("Failed to create symlink: {source} -> {target}")]
    #[diagnostic(code(bombadil::dots::symlink))]
    Symlink {
        source: PathBuf,
        target: PathBuf,
        #[source]
        cause: std::io::Error,
    },

    #[error("Failed to render template: {path}")]
    #[diagnostic(code(bombadil::dots::template))]
    TemplateRender {
        path: PathBuf,
        #[source]
        cause: tera::Error,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Patch Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("Base file not found for patch: {path}")]
    #[diagnostic(
        code(bombadil::patch::base_not_found),
        help("Ensure the base file exists at the specified path")
    )]
    PatchBaseNotFound { path: PathBuf },

    #[error("Patch failed to apply: {patch_file}")]
    #[diagnostic(code(bombadil::patch::apply_failed))]
    PatchApplyFailed {
        patch_file: PathBuf,
        base_file: PathBuf,
        #[help]
        help: String,
    },

    #[error("Invalid patch format in {path}")]
    #[diagnostic(code(bombadil::patch::invalid_format))]
    PatchInvalidFormat {
        path: PathBuf,
        #[help]
        help: String,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Package Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("Package manager not found: {manager}")]
    #[diagnostic(
        code(bombadil::packages::manager_not_found),
        help("Ensure the package manager is installed and in your PATH")
    )]
    PackageManagerNotFound { manager: String },

    #[error("Package not found: {name}")]
    #[diagnostic(code(bombadil::packages::not_found))]
    PackageNotFound { name: String },

    #[error("Package installation failed: {name}")]
    #[diagnostic(code(bombadil::packages::install_failed))]
    PackageInstallFailed {
        name: String,
        manager: String,
        #[source]
        cause: std::io::Error,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Secret Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("GPG user ID not configured")]
    #[diagnostic(
        code(bombadil::secrets::no_gpg_user),
        help("Add `gpg_user_id = \"your-email@example.com\"` to your bombadil.toml")
    )]
    NoGpgUser,

    #[error("GPG decryption failed")]
    #[diagnostic(code(bombadil::secrets::decrypt_failed))]
    GpgDecryptFailed {
        #[source]
        cause: std::io::Error,
    },

    #[error("Unencrypted secret detected in {path}")]
    #[diagnostic(
        code(bombadil::secrets::unencrypted),
        help("Use `bombadil secrets add` to encrypt secrets")
    )]
    UnencryptedSecret { path: PathBuf, pattern: String },

    // ─────────────────────────────────────────────────────────────────────────
    // Conflict Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("Conflict detected: {path}")]
    #[diagnostic(code(bombadil::conflict::detected))]
    ConflictDetected {
        path: PathBuf,
        #[help]
        help: String,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // IO Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("IO error: {context}")]
    #[diagnostic(code(bombadil::io))]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("File operation failed")]
    #[diagnostic(code(bombadil::io::file))]
    FileOperation {
        operation: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Profile Errors
    // ─────────────────────────────────────────────────────────────────────────
    #[error("Profile not found: {name}")]
    #[diagnostic(code(bombadil::profile::not_found))]
    ProfileNotFound {
        name: String,
        #[help]
        available: String,
    },
}

// Convenience constructors
impl BombadilError {
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    pub fn file_op(
        operation: impl Into<String>,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::FileOperation {
            operation: operation.into(),
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = BombadilError::ConfigNotFound {
            path: PathBuf::from("/home/user/.config/bombadil.toml"),
        };
        assert!(err.to_string().contains("Configuration file not found"));
    }

    #[test]
    fn error_has_diagnostic_code() {
        let err = BombadilError::NoGpgUser;
        assert!(err.code().is_some());
    }
}
