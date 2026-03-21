//! Sync orchestrator — builds and executes a `SyncPlan`.
//!
//! # Usage
//!
//! ```no_run
//! use toml_bombadil::sync::{SyncEngine, SyncOptions};
//!
//! let engine = SyncEngine::new("/path/to/bombadil.toml".as_ref());
//! let plan = engine.plan(&SyncOptions::default()).unwrap();
//! let session = engine.execute(plan).unwrap();
//! ```

pub mod plan;

pub use plan::{
    build_sync_plan, DotAction, HookPhase, PackageAction, PlannedDot, PlannedFile, PlannedHook,
    PlannedPackage, SkipReason, SyncItem, SyncOptions, SyncPlan,
};

use crate::core::Result;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Orchestrates the full sync workflow for a bombadil installation.
pub struct SyncEngine {
    /// Path to the `bombadil.toml` config file.
    config_path: PathBuf,
}

impl SyncEngine {
    pub fn new(config_path: &Path) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
        }
    }

    /// Build a sync plan without executing it.
    ///
    /// This is safe to call for `--dry-run` display.
    #[instrument(skip(self, options))]
    pub fn plan(&self, options: &SyncOptions) -> Result<SyncPlan> {
        build_sync_plan(&self.config_path, options)
    }

    /// Execute a previously-built plan.
    ///
    /// Returns `Ok(())` when the plan executed without fatal errors.
    /// Individual item failures are recorded in the audit log but do not
    /// abort the run unless `--fail-fast` (future option) is set.
    #[instrument(skip(self, plan))]
    pub fn execute(&self, plan: SyncPlan) -> Result<()> {
        execute_plan(plan)
    }
}

/// Execute a `SyncPlan`, applying all items in order.
///
/// Each item produces an audit `Action`. The completed set of actions is
/// persisted as a `Session` at the end of the run.
#[instrument(skip(plan))]
pub fn execute_plan(plan: SyncPlan) -> Result<()> {
    use crate::sync::plan::{DotAction, SyncItem};
    use tracing::{info, warn};

    info!(
        dotfiles_dir = %plan.dotfiles_dir.display(),
        items = plan.items.len(),
        "starting sync"
    );

    for item in &plan.items {
        match item {
            SyncItem::Hook(hook) => {
                let owner = hook.owner.as_deref().unwrap_or("global");
                info!(
                    command = %hook.command,
                    owner = %owner,
                    phase = ?hook.phase,
                    "running hook"
                );
                // TODO(Phase 2): execute hook, record Action::HookExecuted
            }
            SyncItem::Dot(dot) => {
                match &dot.action {
                    DotAction::Skip { reason } => {
                        info!(
                            name = %dot.name,
                            root_cause = %reason.root_cause(),
                            "⊘ skipping dot"
                        );
                        continue;
                    }
                    _ => {}
                }

                info!(
                    name = %dot.name,
                    namespace = %dot.namespace,
                    action = %dot.action.indicator(),
                    files = dot.files.len(),
                    "applying dot"
                );

                for file in &dot.files {
                    apply_file(file)?;
                }
            }
            SyncItem::Package(pkg) => {
                info!(
                    name = %pkg.name,
                    action = ?pkg.action,
                    "package"
                );
                // TODO(Phase 3): invoke PackageManager, record Action::PackageInstalled
            }
        }
    }

    info!("sync complete");
    // TODO(Phase 2): persist Session to audit storage
    Ok(())
}

/// Apply a single file mapping (symlink or copy).
fn apply_file(file: &PlannedFile) -> Result<()> {
    use crate::core::BombadilError;
    use std::fs;
    use tracing::{debug, warn};

    if !file.source.exists() {
        warn!(source = %file.source.display(), "source file not found, skipping");
        return Ok(());
    }

    // Ensure target parent directory exists
    if let Some(parent) = file.target.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                context: format!("failed to create directory {}", parent.display()),
                source: e,
            })?;
        }
    }

    match &file.action {
        DotAction::Backup { original } => {
            // Back up unmanaged file before overwriting
            let backup = original.with_extension("bombadil.bak");
            debug!(
                original = %original.display(),
                backup = %backup.display(),
                "backing up existing file"
            );
            if original.exists() {
                fs::rename(original, &backup).map_err(|e| BombadilError::Io {
                    context: format!("failed to backup {}", original.display()),
                    source: e,
                })?;
            }
            create_link_or_copy(file)?;
        }
        DotAction::Update => {
            // Remove old symlink/file before relinking
            if file.target.exists() || file.target.is_symlink() {
                fs::remove_file(&file.target).map_err(|e| BombadilError::Io {
                    context: format!("failed to remove {}", file.target.display()),
                    source: e,
                })?;
            }
            create_link_or_copy(file)?;
        }
        DotAction::Create => {
            create_link_or_copy(file)?;
        }
        DotAction::Unchanged => {
            debug!(target = %file.target.display(), "unchanged, skipping");
        }
        DotAction::Skip { .. } => {}
    }

    Ok(())
}

/// Create a symlink or file copy for a planned file.
fn create_link_or_copy(file: &PlannedFile) -> Result<()> {
    use crate::core::BombadilError;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs as unix_fs;

    if file.copy {
        fs::copy(&file.source, &file.target).map_err(|e| BombadilError::Io {
            context: format!(
                "failed to copy {} → {}",
                file.source.display(),
                file.target.display()
            ),
            source: e,
        })?;
    } else {
        #[cfg(unix)]
        unix_fs::symlink(&file.source, &file.target).map_err(|e| BombadilError::Io {
            context: format!(
                "failed to symlink {} → {}",
                file.source.display(),
                file.target.display()
            ),
            source: e,
        })?;

        #[cfg(not(unix))]
        return Err(BombadilError::ConfigInvalid {
            context: "symlinks require a Unix system".to_string(),
            help: None,
        });
    }
    Ok(())
}
