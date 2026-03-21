//! Package installation helpers (v3 legacy stubs).
//!
//! This module is a compatibility shim. The v3 PackageManager struct in
//! `packages/mod.rs` will be removed in Phase 4; these stubs exist only to
//! satisfy the compiler until that cleanup happens.

use crate::platform::PlatformContext;
use crate::settings::packages::{BinaryConfig, GitConfig, SourceConfig};
use anyhow::{bail, Result};
use std::path::Path;

/// Run a command with arguments, waiting for it to finish.
pub fn run_command(cmd: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(cmd)
        .args(args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        bail!("command `{} {}` failed with status {}", cmd, args.join(" "), status)
    }
}

/// Run two commands piped together.
pub fn run_command_piped(cmd1: &[&str], cmd2: &[&str]) -> Result<()> {
    let (c1, a1) = cmd1.split_first().ok_or_else(|| anyhow::anyhow!("empty command"))?;
    let (c2, a2) = cmd2.split_first().ok_or_else(|| anyhow::anyhow!("empty command"))?;

    let child1 = std::process::Command::new(c1)
        .args(a1)
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    let status = std::process::Command::new(c2)
        .args(a2)
        .stdin(child1.stdout.unwrap())
        .status()?;

    if status.success() {
        Ok(())
    } else {
        bail!("piped command failed with status {}", status)
    }
}

/// Install a binary package from a URL or GitHub release.
#[allow(unused_variables)]
pub fn install_binary(
    name: &str,
    config: &BinaryConfig,
    platform: &PlatformContext,
) -> Result<Option<String>> {
    bail!("binary install not yet implemented in this build")
}

/// Install a package from a git repository.
#[allow(unused_variables)]
pub fn install_from_git(
    name: &str,
    config: &GitConfig,
    dotfiles_path: &Path,
) -> Result<Option<String>> {
    bail!("git install not yet implemented in this build")
}

/// Install a package from local source.
#[allow(unused_variables)]
pub fn install_from_source(name: &str, config: &SourceConfig) -> Result<Option<String>> {
    bail!("source install not yet implemented in this build")
}
