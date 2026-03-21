//! Action executor for performing planned actions.
//!
//! The executor takes a plan and executes each approved action,
//! recording results and saving content snapshots for revert.

use super::action::{Action, ActionType};
use super::plan::{ActionPlan, PlannedAction};
use super::session::Session;
use super::storage::{content_hash, AuditStorage};
use anyhow::{Context, Result};
use std::fs;
use std::os::unix;
use std::path::Path;
use tracing::{debug, info, warn};

/// Result of executing a single action.
#[derive(Debug)]
pub struct ExecutionResult {
    /// The action that was executed.
    pub action: Action,
    /// Whether the action succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Executor for performing actions.
pub struct ActionExecutor {
    storage: AuditStorage,
    dry_run: bool,
}

impl ActionExecutor {
    /// Create a new executor.
    pub fn new(storage: AuditStorage, dry_run: bool) -> Self {
        Self { storage, dry_run }
    }

    /// Execute a full plan, returning the session with all actions.
    pub fn execute_plan(
        &self,
        plan: &ActionPlan,
        command: &str,
        profiles: Vec<String>,
    ) -> Result<Session> {
        let mut session = Session::new(command, profiles);

        // Initialize storage
        if !self.dry_run {
            self.storage.init()?;
        }

        for planned in plan.approved_actions() {
            let result = self.execute_action(planned)?;
            if result.success {
                session.add_action(result.action);
            } else if let Some(error) = result.error {
                warn!(error = %error, "Action failed");
            }
        }

        session.complete();

        // Save session if not dry run
        if !self.dry_run && !session.is_empty() {
            self.storage.save_session(&session)?;
        }

        Ok(session)
    }

    /// Execute a single planned action.
    pub fn execute_action(&self, planned: &PlannedAction) -> Result<ExecutionResult> {
        let action = planned.clone().into_action();

        if self.dry_run {
            return Ok(ExecutionResult {
                action,
                success: true,
                error: None,
            });
        }

        let result = match &action.action_type {
            ActionType::FileCreate {
                target,
                source,
                content_hash: _,
            } => self.execute_file_create(target, source, &action),

            ActionType::FileUpdate {
                target,
                source,
                before_hash: _,
                after_hash: _,
            } => self.execute_file_update(target, source, &action),

            ActionType::SymlinkCreate { source, target } => {
                self.execute_symlink_create(source, target, &action)
            }

            ActionType::SymlinkRemove {
                target,
                was_pointing_to: _,
            } => self.execute_symlink_remove(target, &action),

            ActionType::Backup {
                original,
                backup_location,
                content_hash: _,
            } => self.execute_backup(original, backup_location, &action),

            // These action types need content from elsewhere
            ActionType::FilePatch { .. }
            | ActionType::SemanticPatch { .. }
            | ActionType::Inject { .. } => {
                // These are handled by the dot strategies directly
                Ok(ExecutionResult {
                    action,
                    success: true,
                    error: None,
                })
            }

            ActionType::ConflictResolved { .. } => {
                // Conflict resolution is handled separately
                Ok(ExecutionResult {
                    action,
                    success: true,
                    error: None,
                })
            }

            ActionType::HookExecuted { .. } => {
                // Hooks are executed separately
                Ok(ExecutionResult {
                    action,
                    success: true,
                    error: None,
                })
            }
        };

        result
    }

    /// Execute a file create action.
    fn execute_file_create(
        &self,
        target: &Path,
        source: &Path,
        action: &Action,
    ) -> Result<ExecutionResult> {
        debug!(target = ?target, source = ?source, "Creating file");

        // Read source content
        let content = fs::read(source)
            .with_context(|| format!("Failed to read source: {}", source.display()))?;

        // Save after content for revert
        self.storage.save_after_content(&action.id, &content)?;

        // Create parent directories if needed
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }

        // Write to target
        fs::write(target, &content)
            .with_context(|| format!("Failed to write target: {}", target.display()))?;

        info!(target = ?target, "Created file");

        Ok(ExecutionResult {
            action: action.clone(),
            success: true,
            error: None,
        })
    }

    /// Execute a file update action.
    fn execute_file_update(
        &self,
        target: &Path,
        source: &Path,
        action: &Action,
    ) -> Result<ExecutionResult> {
        debug!(target = ?target, source = ?source, "Updating file");

        // Save before content for revert
        if target.exists() {
            let before = fs::read(target)
                .with_context(|| format!("Failed to read target: {}", target.display()))?;
            self.storage.save_before_content(&action.id, &before)?;
        }

        // Read source content
        let content = fs::read(source)
            .with_context(|| format!("Failed to read source: {}", source.display()))?;

        // Save after content for revert
        self.storage.save_after_content(&action.id, &content)?;

        // Write to target
        fs::write(target, &content)
            .with_context(|| format!("Failed to write target: {}", target.display()))?;

        info!(target = ?target, "Updated file");

        Ok(ExecutionResult {
            action: action.clone(),
            success: true,
            error: None,
        })
    }

    /// Execute a symlink create action.
    fn execute_symlink_create(
        &self,
        source: &Path,
        target: &Path,
        action: &Action,
    ) -> Result<ExecutionResult> {
        debug!(source = ?source, target = ?target, "Creating symlink");

        // Create parent directories if needed
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }

        // Remove existing target if it exists
        if target.exists() || target.symlink_metadata().is_ok() {
            fs::remove_file(target)
                .with_context(|| format!("Failed to remove existing target: {}", target.display()))?;
        }

        // Create symlink
        unix::fs::symlink(source, target)
            .with_context(|| format!("Failed to create symlink: {} -> {}", source.display(), target.display()))?;

        info!(source = ?source, target = ?target, "Created symlink");

        Ok(ExecutionResult {
            action: action.clone(),
            success: true,
            error: None,
        })
    }

    /// Execute a symlink remove action.
    fn execute_symlink_remove(&self, target: &Path, action: &Action) -> Result<ExecutionResult> {
        debug!(target = ?target, "Removing symlink");

        if target.symlink_metadata().is_ok() {
            fs::remove_file(target)
                .with_context(|| format!("Failed to remove symlink: {}", target.display()))?;
            info!(target = ?target, "Removed symlink");
        }

        Ok(ExecutionResult {
            action: action.clone(),
            success: true,
            error: None,
        })
    }

    /// Execute a backup action.
    fn execute_backup(
        &self,
        original: &Path,
        backup_location: &Path,
        action: &Action,
    ) -> Result<ExecutionResult> {
        debug!(original = ?original, backup = ?backup_location, "Backing up file");

        // Read original content
        let content = fs::read(original)
            .with_context(|| format!("Failed to read original: {}", original.display()))?;

        // Save content for audit trail
        self.storage.save_before_content(&action.id, &content)?;

        // Create backup directory if needed
        if let Some(parent) = backup_location.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create backup directory: {}", parent.display()))?;
            }
        }

        // Copy to backup location
        fs::write(backup_location, &content)
            .with_context(|| format!("Failed to write backup: {}", backup_location.display()))?;

        info!(original = ?original, backup = ?backup_location, "Backed up file");

        Ok(ExecutionResult {
            action: action.clone(),
            success: true,
            error: None,
        })
    }

    /// Record a file operation with content snapshots.
    ///
    /// This is called by dot strategies to record their actions.
    pub fn record_file_change(
        &self,
        action: &Action,
        before_content: Option<&[u8]>,
        after_content: &[u8],
    ) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }

        if let Some(before) = before_content {
            self.storage.save_before_content(&action.id, before)?;
        }
        self.storage.save_after_content(&action.id, after_content)?;

        Ok(())
    }
}

/// Helper to create actions for common operations.
pub mod helpers {
    use super::*;
    use std::path::PathBuf;

    /// Create a FileCreate action.
    pub fn file_create(target: PathBuf, source: PathBuf, content: &[u8]) -> ActionType {
        ActionType::FileCreate {
            target,
            source,
            content_hash: content_hash(content),
        }
    }

    /// Create a FileUpdate action.
    pub fn file_update(
        target: PathBuf,
        source: PathBuf,
        before: &[u8],
        after: &[u8],
    ) -> ActionType {
        ActionType::FileUpdate {
            target,
            source,
            before_hash: content_hash(before),
            after_hash: content_hash(after),
        }
    }

    /// Create a SymlinkCreate action.
    pub fn symlink_create(source: PathBuf, target: PathBuf) -> ActionType {
        ActionType::SymlinkCreate { source, target }
    }

    /// Create a SymlinkRemove action.
    pub fn symlink_remove(target: PathBuf, was_pointing_to: PathBuf) -> ActionType {
        ActionType::SymlinkRemove {
            target,
            was_pointing_to,
        }
    }

    /// Create a Backup action.
    pub fn backup(original: PathBuf, backup_location: PathBuf, content: &[u8]) -> ActionType {
        ActionType::Backup {
            original,
            backup_location,
            content_hash: content_hash(content),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_dry_run_execution() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let executor = ActionExecutor::new(storage, true);

        let mut plan = ActionPlan::new();
        plan.add_action(
            ActionType::FileCreate {
                target: dir.path().join("test.txt"),
                source: dir.path().join("source.txt"),
                content_hash: "abc".to_string(),
            },
            None,
        );

        let session = executor.execute_plan(&plan, "link", vec![]).unwrap();
        assert_eq!(session.actions.len(), 1);

        // File should not actually be created in dry run
        assert!(!dir.path().join("test.txt").exists());
    }
}
