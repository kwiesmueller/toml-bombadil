//! Package config persistence helpers (v3 legacy stub).
//!
//! Will be replaced/removed in Phase 4 along with the rest of the v3
//! package management code.

use crate::settings::packages::Package;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Return the default path for the packages config file.
pub fn default_packages_file(dotfiles_path: &Path) -> PathBuf {
    dotfiles_path.join("packages.toml")
}

/// Append a package definition to a TOML config file.
#[allow(unused_variables)]
pub fn append_package_to_file(
    name: &str,
    package: &Package,
    target_file: &Path,
    dotfiles_path: &Path,
) -> Result<()> {
    bail!("append_package_to_file not yet implemented in this build")
}

/// Interactively prompt the user to configure a package and write it to file.
#[allow(unused_variables)]
pub fn interactive_add_package(
    name: &str,
    tags: &[String],
    target_file: &Path,
    dotfiles_path: &Path,
) -> Result<Package> {
    bail!("interactive_add_package not yet implemented in this build")
}
