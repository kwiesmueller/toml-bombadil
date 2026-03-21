//! Dotfile management strategies.
//!
//! This module provides different strategies for managing dotfiles:
//! - Full replacement (symlinks)
//! - Line-based patching
//! - Semantic patching (for structured formats)
//! - Inject (append/prepend)

pub mod render;
pub mod strategy;

// Re-export strategy types
pub use strategy::{DotInstaller, InstallResult};
