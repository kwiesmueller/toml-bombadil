//! Dotfile installation strategies.

pub mod full;
pub mod inject;
pub mod patch;
pub mod semantic;

use crate::config::Dot;
use crate::core::Result;
use std::path::Path;

/// Result of installing a dotfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallResult {
    /// New symlink/file created.
    Created,
    /// Existing file updated.
    Updated,
    /// File unchanged (already up to date).
    Unchanged,
    /// File ignored (matched ignore pattern).
    Ignored,
    /// File skipped (conflict resolution).
    Skipped,
}

/// Trait for dotfile installation strategies.
pub trait DotInstaller {
    /// Install the dotfile using this strategy.
    fn install(
        &self,
        dot: &Dot,
        dotfiles_dir: &Path,
        vars: &tera::Context,
    ) -> Result<InstallResult>;
}
