//! Revert engine for undoing actions.
//!
//! Allows users to revert individual actions by restoring the "before" state.
//! Supports dependency-aware revert with full analysis of blockers and warnings.

use super::action::{Action, ActionId, ActionType};
use super::file_index::{FileAction, FileIndex};
use super::storage::AuditStorage;
use anyhow::{Context, Result};
use std::fs;
use std::os::unix;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Result of a revert operation.
#[derive(Debug)]
pub struct RevertResult {
    /// The action that was reverted.
    pub action_id: ActionId,
    /// Whether the revert succeeded.
    pub success: bool,
    /// Description of what was done.
    pub message: String,
    /// Warning if the file was modified after the action.
    pub modified_warning: bool,
}

/// Full analysis of reverting an action
#[derive(Debug)]
pub struct RevertAnalysis {
    /// The action being analyzed
    pub action: Action,
    /// Whether the action can be reverted
    pub can_revert: bool,
    /// Conditions that block the revert
    pub blockers: Vec<RevertBlocker>,
    /// Warnings about reverting the action
    pub warnings: Vec<RevertWarning>,
}

/// Conditions that block a revert
#[derive(Debug)]
pub enum RevertBlocker {
    /// File was modified externally since the action
    FileModifiedExternally {
        path: PathBuf,
        expected_hash: String,
        current_hash: String,
    },
    /// Another action depends on this one
    DependentActionExists {
        action_id: ActionId,
        description: String,
    },
    /// Content needed for revert is missing
    ContentMissing { hash: String },
    /// Action type is not revertible
    NotRevertible { reason: String },
}

/// Warnings about reverting an action
#[derive(Debug)]
pub enum RevertWarning {
    /// Subsequent actions exist on the same file
    SubsequentActionsExist { actions: Vec<ActionId> },
    /// Reverting will invalidate another action
    WillInvalidateAction { action_id: ActionId, reason: String },
}

/// Options for file-level revert
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct FileRevertOptions {
    /// Force revert even if file was modified
    pub force: bool,
    /// Don't actually perform the revert, just show what would happen
    pub dry_run: bool,
    /// Continue reverting remaining actions if one fails
    pub continue_on_error: bool,
    /// Revert to a specific action's state (None means revert all)
    pub to_action_id: Option<ActionId>,
}


/// Engine for reverting actions.
pub struct RevertEngine {
    storage: AuditStorage,
    file_index: FileIndex,
}

impl RevertEngine {
    /// Create a new revert engine.
    pub fn new(storage: AuditStorage) -> Self {
        Self {
            storage,
            file_index: FileIndex::new(),
        }
    }

    /// Create a new revert engine with a file index.
    pub fn with_file_index(storage: AuditStorage, file_index: FileIndex) -> Self {
        Self {
            storage,
            file_index,
        }
    }

    /// Get reference to the file index.
    pub fn file_index(&self) -> &FileIndex {
        &self.file_index
    }

    /// Check if an action can be reverted.
    pub fn can_revert(&self, action: &Action) -> RevertCheck {
        // Check if action type is revertible
        if !action.is_revertible() {
            return RevertCheck::NotRevertible {
                reason: format!(
                    "Action type '{}' cannot be reverted",
                    action.action_type.type_indicator()
                ),
            };
        }

        // Check if we have the content needed for revert
        match &action.action_type {
            ActionType::FileCreate { target, .. } => {
                // Can revert by deleting the file
                if !target.exists() {
                    return RevertCheck::AlreadyReverted;
                }
                RevertCheck::CanRevert
            }

            ActionType::FileUpdate {
                target, after_hash, ..
            }
            | ActionType::FilePatch {
                target, after_hash, ..
            }
            | ActionType::SemanticPatch {
                target, after_hash, ..
            }
            | ActionType::Inject {
                target, after_hash, ..
            } => {
                // Need before content to revert
                if !self.storage.has_content(&action.id) {
                    return RevertCheck::MissingContent {
                        reason: "Before content not available".to_string(),
                    };
                }

                // Check if file was modified after the action
                if target.exists() {
                    if let Ok(current) = fs::read(target) {
                        let current_hash = super::storage::content_hash(&current);
                        if current_hash != *after_hash {
                            return RevertCheck::ModifiedSinceAction {
                                current_hash,
                                expected_hash: after_hash.clone(),
                            };
                        }
                    }
                }

                RevertCheck::CanRevert
            }

            ActionType::SymlinkCreate { target, source } => {
                // Can revert by removing the symlink
                if target.symlink_metadata().is_err() {
                    return RevertCheck::AlreadyReverted;
                }

                // Verify symlink still points to expected source
                if let Ok(link_target) = fs::read_link(target) {
                    if link_target != *source {
                        return RevertCheck::ModifiedSinceAction {
                            current_hash: link_target.to_string_lossy().to_string(),
                            expected_hash: source.to_string_lossy().to_string(),
                        };
                    }
                }

                RevertCheck::CanRevert
            }

            ActionType::SymlinkRemove {
                target,
                was_pointing_to,
            } => {
                // Can revert by recreating the symlink
                if target.symlink_metadata().is_ok() {
                    return RevertCheck::AlreadyReverted;
                }

                // Check if the original source still exists
                if !was_pointing_to.exists() {
                    return RevertCheck::MissingContent {
                        reason: format!(
                            "Original symlink target no longer exists: {}",
                            was_pointing_to.display()
                        ),
                    };
                }

                RevertCheck::CanRevert
            }

            ActionType::Backup {
                backup_location, ..
            } => {
                // Can revert by moving backup back to original
                if !backup_location.exists() {
                    return RevertCheck::MissingContent {
                        reason: format!(
                            "Backup file no longer exists: {}",
                            backup_location.display()
                        ),
                    };
                }
                RevertCheck::CanRevert
            }

            _ => RevertCheck::NotRevertible {
                reason: "Action type not supported for revert".to_string(),
            },
        }
    }

    /// Revert an action.
    pub fn revert(&self, action: &Action, force: bool) -> Result<RevertResult> {
        let check = self.can_revert(action);

        match &check {
            RevertCheck::NotRevertible { reason } => {
                return Ok(RevertResult {
                    action_id: action.id.clone(),
                    success: false,
                    message: reason.clone(),
                    modified_warning: false,
                });
            }
            RevertCheck::AlreadyReverted => {
                return Ok(RevertResult {
                    action_id: action.id.clone(),
                    success: true,
                    message: "Already reverted".to_string(),
                    modified_warning: false,
                });
            }
            RevertCheck::MissingContent { reason } => {
                return Ok(RevertResult {
                    action_id: action.id.clone(),
                    success: false,
                    message: reason.clone(),
                    modified_warning: false,
                });
            }
            RevertCheck::ModifiedSinceAction { .. } if !force => {
                return Ok(RevertResult {
                    action_id: action.id.clone(),
                    success: false,
                    message: "File was modified after this action. Use --force to revert anyway."
                        .to_string(),
                    modified_warning: true,
                });
            }
            _ => {}
        }

        let modified_warning = matches!(check, RevertCheck::ModifiedSinceAction { .. });

        // Perform the revert
        let message = match &action.action_type {
            ActionType::FileCreate { target, .. } => {
                self.revert_file_create(target)?;
                format!("Deleted {}", target.display())
            }

            ActionType::FileUpdate { target, .. }
            | ActionType::FilePatch { target, .. }
            | ActionType::SemanticPatch { target, .. }
            | ActionType::Inject { target, .. } => {
                self.revert_file_update(&action.id, target)?;
                format!("Restored {}", target.display())
            }

            ActionType::SymlinkCreate { target, .. } => {
                self.revert_symlink_create(target)?;
                format!("Removed symlink {}", target.display())
            }

            ActionType::SymlinkRemove {
                target,
                was_pointing_to,
            } => {
                self.revert_symlink_remove(target, was_pointing_to)?;
                format!("Restored symlink {}", target.display())
            }

            ActionType::Backup {
                original,
                backup_location,
                ..
            } => {
                self.revert_backup(original, backup_location)?;
                format!("Restored {} from backup", original.display())
            }

            _ => unreachable!(),
        };

        info!(action_id = %action.id, "Reverted action");

        Ok(RevertResult {
            action_id: action.id.clone(),
            success: true,
            message,
            modified_warning,
        })
    }

    /// Analyze if an action can be reverted with full dependency checking.
    ///
    /// This provides a comprehensive analysis including:
    /// - Whether the action can be reverted
    /// - Any blockers that prevent revert
    /// - Warnings about subsequent actions on the same file
    pub fn analyze(&self, action: &Action) -> Result<RevertAnalysis> {
        let mut blockers = Vec::new();
        let mut warnings = Vec::new();

        // Check basic revertibility using existing logic
        let check = self.can_revert(action);

        match &check {
            RevertCheck::NotRevertible { reason } => {
                blockers.push(RevertBlocker::NotRevertible {
                    reason: reason.clone(),
                });
            }
            RevertCheck::MissingContent { reason } => {
                // Extract hash from the action if available
                let hash = self.get_before_hash(action).unwrap_or_default();
                blockers.push(RevertBlocker::ContentMissing { hash });
                debug!("Content missing for revert: {}", reason);
            }
            RevertCheck::ModifiedSinceAction {
                current_hash,
                expected_hash,
            } => {
                if let Some(target) = action.action_type.target_path() {
                    blockers.push(RevertBlocker::FileModifiedExternally {
                        path: target.clone(),
                        expected_hash: expected_hash.clone(),
                        current_hash: current_hash.clone(),
                    });
                }
            }
            RevertCheck::CanRevert | RevertCheck::AlreadyReverted => {
                // No blockers
            }
        }

        // Check for subsequent actions on the same file using the file index
        if let Some(target) = action.action_type.target_path() {
            let subsequent = self.file_index.actions_after(target, &action.id);
            if !subsequent.is_empty() {
                let action_ids: Vec<ActionId> =
                    subsequent.iter().map(|fa| fa.action_id.clone()).collect();
                warnings.push(RevertWarning::SubsequentActionsExist {
                    actions: action_ids.clone(),
                });

                // Each subsequent action would be invalidated
                for fa in subsequent {
                    warnings.push(RevertWarning::WillInvalidateAction {
                        action_id: fa.action_id.clone(),
                        reason: "This action was applied after the action being reverted".to_string(),
                    });
                }
            }
        }

        let can_revert = blockers.is_empty()
            || blockers
                .iter()
                .all(|b| matches!(b, RevertBlocker::FileModifiedExternally { .. }));

        Ok(RevertAnalysis {
            action: action.clone(),
            can_revert,
            blockers,
            warnings,
        })
    }

    /// Get all actions that would be affected by reverting this action.
    ///
    /// Returns action IDs for all subsequent actions on the same file.
    pub fn dependent_actions(&self, action: &Action) -> Result<Vec<ActionId>> {
        let mut dependent = Vec::new();

        if let Some(target) = action.action_type.target_path() {
            let subsequent = self.file_index.actions_after(target, &action.id);
            for fa in subsequent {
                dependent.push(fa.action_id.clone());
            }
        }

        Ok(dependent)
    }

    /// Revert all actions on a file, newest first.
    ///
    /// This retrieves all actions for the given file from the file index
    /// and reverts them in reverse chronological order (newest first).
    pub fn revert_file(
        &self,
        path: &Path,
        options: FileRevertOptions,
    ) -> Result<Vec<RevertResult>> {
        let mut results = Vec::new();

        // Get all actions for this file
        let file_actions = match self.file_index.get(path) {
            Some(actions) => actions.clone(),
            None => {
                debug!("No actions found for file: {}", path.display());
                return Ok(results);
            }
        };

        if file_actions.is_empty() {
            return Ok(results);
        }

        // Determine which actions to revert based on to_action_id
        let actions_to_revert: Vec<&FileAction> = if let Some(ref target_id) = options.to_action_id
        {
            // Find actions newer than the target
            self.file_index.actions_after(path, target_id)
        } else {
            // Revert all actions (in reverse order)
            file_actions.iter().collect()
        };

        // Process in reverse order (newest first)
        let mut actions_rev: Vec<_> = actions_to_revert.into_iter().collect();
        actions_rev.reverse();

        for file_action in actions_rev {
            // Load the full action from storage
            let action = match self.load_action(&file_action.action_id) {
                Ok(Some(action)) => action,
                Ok(None) => {
                    warn!(
                        action_id = %file_action.action_id,
                        "Action not found in storage"
                    );
                    if !options.continue_on_error {
                        return Err(anyhow::anyhow!(
                            "Action {} not found in storage",
                            file_action.action_id
                        ));
                    }
                    continue;
                }
                Err(e) => {
                    warn!(
                        action_id = %file_action.action_id,
                        error = %e,
                        "Failed to load action"
                    );
                    if !options.continue_on_error {
                        return Err(e);
                    }
                    continue;
                }
            };

            if options.dry_run {
                // Just analyze without performing revert
                let analysis = self.analyze(&action)?;
                results.push(RevertResult {
                    action_id: action.id.clone(),
                    success: analysis.can_revert,
                    message: if analysis.can_revert {
                        format!(
                            "[dry-run] Would revert: {}",
                            action.action_type.description()
                        )
                    } else {
                        format!(
                            "[dry-run] Cannot revert: {} blockers",
                            analysis.blockers.len()
                        )
                    },
                    modified_warning: !analysis.warnings.is_empty(),
                });
            } else {
                // Perform actual revert
                let result = self.revert(&action, options.force)?;
                let success = result.success;
                results.push(result);

                if !success && !options.continue_on_error {
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Revert file to a specific action's state.
    ///
    /// This finds all actions newer than the target action and reverts them
    /// in order, leaving the file in the state it was after the target action.
    pub fn revert_file_to(
        &self,
        path: &Path,
        action_id: &ActionId,
        options: FileRevertOptions,
    ) -> Result<Vec<RevertResult>> {
        // Validate that the action exists for this file
        let file_actions = self.file_index.get(path);
        let action_exists = file_actions
            .map(|actions| actions.iter().any(|a| &a.action_id == action_id))
            .unwrap_or(false);

        if !action_exists {
            return Err(anyhow::anyhow!(
                "Action {} not found for file {}",
                action_id,
                path.display()
            ));
        }

        // Use revert_file with the to_action_id option set
        let mut opts = options;
        opts.to_action_id = Some(action_id.clone());
        self.revert_file(path, opts)
    }

    /// Helper to get the before hash from an action type.
    fn get_before_hash(&self, action: &Action) -> Option<String> {
        match &action.action_type {
            ActionType::FileUpdate { before_hash, .. }
            | ActionType::FilePatch { before_hash, .. }
            | ActionType::SemanticPatch { before_hash, .. }
            | ActionType::Inject { before_hash, .. } => Some(before_hash.clone()),
            _ => None,
        }
    }

    /// Helper to load a full action from storage by ID.
    fn load_action(&self, action_id: &ActionId) -> Result<Option<Action>> {
        match self.storage.find_session_for_action(action_id)? {
            Some(session) => {
                let action = session.actions.into_iter().find(|a| &a.id == action_id);
                Ok(action)
            }
            None => Ok(None),
        }
    }

    /// Revert a file create action by deleting the file.
    fn revert_file_create(&self, target: &Path) -> Result<()> {
        debug!(target = ?target, "Reverting file create");
        fs::remove_file(target)
            .with_context(|| format!("Failed to delete file: {}", target.display()))?;
        Ok(())
    }

    /// Revert a file update action by restoring the before content.
    fn revert_file_update(&self, action_id: &ActionId, target: &Path) -> Result<()> {
        debug!(target = ?target, "Reverting file update");

        let before_content = self.storage.load_before_content(action_id)?;
        fs::write(target, before_content)
            .with_context(|| format!("Failed to restore file: {}", target.display()))?;
        Ok(())
    }

    /// Revert a symlink create action by removing the symlink.
    fn revert_symlink_create(&self, target: &Path) -> Result<()> {
        debug!(target = ?target, "Reverting symlink create");
        fs::remove_file(target)
            .with_context(|| format!("Failed to remove symlink: {}", target.display()))?;
        Ok(())
    }

    /// Revert a symlink remove action by recreating the symlink.
    fn revert_symlink_remove(&self, target: &Path, was_pointing_to: &Path) -> Result<()> {
        debug!(target = ?target, was_pointing_to = ?was_pointing_to, "Reverting symlink remove");

        // Create parent directories if needed
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }

        unix::fs::symlink(was_pointing_to, target)
            .with_context(|| format!("Failed to create symlink: {}", target.display()))?;
        Ok(())
    }

    /// Revert a backup action by restoring the backup to the original location.
    fn revert_backup(&self, original: &Path, backup_location: &Path) -> Result<()> {
        debug!(original = ?original, backup = ?backup_location, "Reverting backup");

        // Create parent directories if needed
        if let Some(parent) = original.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }

        fs::copy(backup_location, original)
            .with_context(|| format!("Failed to restore backup: {}", original.display()))?;
        Ok(())
    }
}

/// Result of checking if an action can be reverted.
#[derive(Debug)]
pub enum RevertCheck {
    /// Action can be reverted.
    CanRevert,
    /// Action type doesn't support revert.
    NotRevertible { reason: String },
    /// Action appears to have already been reverted.
    AlreadyReverted,
    /// Content needed for revert is missing.
    MissingContent { reason: String },
    /// File was modified after the action was performed.
    ModifiedSinceAction {
        current_hash: String,
        expected_hash: String,
    },
}

impl RevertCheck {
    /// Check if the action can be reverted (possibly with force).
    pub fn can_proceed(&self, force: bool) -> bool {
        match self {
            RevertCheck::CanRevert => true,
            RevertCheck::AlreadyReverted => true,
            RevertCheck::ModifiedSinceAction { .. } => force,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::action::Action;
    use crate::audit::file_index::FileActionType;
    use crate::audit::session::SessionId;
    use chrono::Utc;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_revert_check_file_create() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let engine = RevertEngine::new(storage);

        // Create a file
        let target = dir.path().join("test.txt");
        fs::write(&target, "content").unwrap();

        let action = Action::new(
            ActionType::FileCreate {
                target: target.clone(),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            None,
        );

        let check = engine.can_revert(&action);
        assert!(matches!(check, RevertCheck::CanRevert));
    }

    #[test]
    fn test_revert_check_already_reverted() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let engine = RevertEngine::new(storage);

        // Don't create the file
        let target = dir.path().join("nonexistent.txt");

        let action = Action::new(
            ActionType::FileCreate {
                target,
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            None,
        );

        let check = engine.can_revert(&action);
        assert!(matches!(check, RevertCheck::AlreadyReverted));
    }

    #[test]
    fn test_file_revert_options_default() {
        let options = FileRevertOptions::default();
        assert!(!options.force);
        assert!(!options.dry_run);
        assert!(!options.continue_on_error);
        assert!(options.to_action_id.is_none());
    }

    #[test]
    fn test_analyze_revertible_action() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let engine = RevertEngine::new(storage);

        // Create a file that exists
        let target = dir.path().join("test.txt");
        fs::write(&target, "content").unwrap();

        let action = Action::new(
            ActionType::FileCreate {
                target: target.clone(),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            None,
        );

        let analysis = engine.analyze(&action).unwrap();
        assert!(analysis.can_revert);
        assert!(analysis.blockers.is_empty());
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn test_analyze_not_revertible_action() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let engine = RevertEngine::new(storage);

        let action = Action::new(
            ActionType::HookExecuted {
                command: "echo test".to_string(),
                hook_type: crate::audit::action::HookType::PostInstall,
                exit_code: 0,
            },
            None,
        );

        let analysis = engine.analyze(&action).unwrap();
        assert!(!analysis.can_revert);
        assert!(!analysis.blockers.is_empty());
        assert!(matches!(
            &analysis.blockers[0],
            RevertBlocker::NotRevertible { .. }
        ));
    }

    #[test]
    fn test_analyze_with_subsequent_actions() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());

        // Create a file index with multiple actions on the same file
        let mut file_index = FileIndex::new();
        let target = dir.path().join("test.txt");

        let action_id_1 = ActionId::from_string("act01");
        let action_id_2 = ActionId::from_string("act02");
        let action_id_3 = ActionId::from_string("act03");

        let timestamp = Utc::now();
        let session_id = SessionId::from_string("test_session");

        // Add actions in chronological order
        file_index.add(
            target.clone(),
            FileAction {
                action_id: action_id_1.clone(),
                session_id: session_id.clone(),
                timestamp,
                action_type: FileActionType::Create,
                content_hash: Some("h1".to_string()),
            },
        );
        file_index.add(
            target.clone(),
            FileAction {
                action_id: action_id_2.clone(),
                session_id: session_id.clone(),
                timestamp,
                action_type: FileActionType::Update,
                content_hash: Some("h2".to_string()),
            },
        );
        file_index.add(
            target.clone(),
            FileAction {
                action_id: action_id_3.clone(),
                session_id: session_id.clone(),
                timestamp,
                action_type: FileActionType::Update,
                content_hash: Some("h3".to_string()),
            },
        );

        let engine = RevertEngine::with_file_index(storage, file_index);

        // Create the file so it can be reverted
        fs::write(&target, "content").unwrap();

        // Create an action with the first action ID
        let action = Action {
            id: action_id_1.clone(),
            timestamp,
            action_type: ActionType::FileCreate {
                target: target.clone(),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            dot_name: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            log_captured: false,
        };

        let analysis = engine.analyze(&action).unwrap();

        // Should have warnings about subsequent actions
        assert!(!analysis.warnings.is_empty());

        let has_subsequent_warning = analysis.warnings.iter().any(|w| {
            matches!(w, RevertWarning::SubsequentActionsExist { actions } if actions.len() == 2)
        });
        assert!(has_subsequent_warning);
    }

    #[test]
    fn test_dependent_actions() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());

        let mut file_index = FileIndex::new();
        let target = dir.path().join("test.txt");

        let action_id_1 = ActionId::from_string("act01");
        let action_id_2 = ActionId::from_string("act02");
        let action_id_3 = ActionId::from_string("act03");

        let timestamp = Utc::now();
        let session_id = SessionId::from_string("test_session");

        file_index.add(
            target.clone(),
            FileAction {
                action_id: action_id_1.clone(),
                session_id: session_id.clone(),
                timestamp,
                action_type: FileActionType::Create,
                content_hash: Some("h1".to_string()),
            },
        );
        file_index.add(
            target.clone(),
            FileAction {
                action_id: action_id_2.clone(),
                session_id: session_id.clone(),
                timestamp,
                action_type: FileActionType::Update,
                content_hash: Some("h2".to_string()),
            },
        );
        file_index.add(
            target.clone(),
            FileAction {
                action_id: action_id_3.clone(),
                session_id: session_id.clone(),
                timestamp,
                action_type: FileActionType::Update,
                content_hash: Some("h3".to_string()),
            },
        );

        let engine = RevertEngine::with_file_index(storage, file_index);

        let action = Action {
            id: action_id_1.clone(),
            timestamp,
            action_type: ActionType::FileCreate {
                target: target.clone(),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            dot_name: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            log_captured: false,
        };

        let dependent = engine.dependent_actions(&action).unwrap();
        assert_eq!(dependent.len(), 2);
        assert_eq!(dependent[0].as_str(), "act02");
        assert_eq!(dependent[1].as_str(), "act03");
    }

    #[test]
    fn test_revert_file_no_actions() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let engine = RevertEngine::new(storage);

        let path = dir.path().join("unknown.txt");
        let options = FileRevertOptions::default();

        let results = engine.revert_file(&path, options).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_revert_file_to_invalid_action() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let engine = RevertEngine::new(storage);

        let path = dir.path().join("test.txt");
        let action_id = ActionId::from_string("nonexistent");
        let options = FileRevertOptions::default();

        let result = engine.revert_file_to(&path, &action_id, options);
        assert!(result.is_err());
    }

    #[test]
    fn test_analyze_modified_externally() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        // Create a file and save content
        let target = dir.path().join("test.txt");
        fs::write(&target, "original content").unwrap();

        let action_id = ActionId::from_string("test_action");
        storage
            .save_before_content(&action_id, b"before content")
            .unwrap();

        // Create an action with a different expected hash
        let action = Action {
            id: action_id,
            timestamp: Utc::now(),
            action_type: ActionType::FileUpdate {
                target: target.clone(),
                source: PathBuf::from("/src"),
                before_hash: "before_hash".to_string(),
                after_hash: "different_hash".to_string(), // Doesn't match current file
            },
            dot_name: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            log_captured: false,
        };

        let engine = RevertEngine::new(storage);
        let analysis = engine.analyze(&action).unwrap();

        // Should have a blocker for external modification
        // can_revert is true because FileModifiedExternally is not a hard blocker
        assert!(analysis.can_revert);
        assert!(!analysis.blockers.is_empty());
        assert!(matches!(
            &analysis.blockers[0],
            RevertBlocker::FileModifiedExternally { .. }
        ));
    }

    #[test]
    fn test_with_file_index_constructor() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        let file_index = FileIndex::new();

        let engine = RevertEngine::with_file_index(storage, file_index);

        // Verify we can access the file index
        assert!(engine.file_index().files.is_empty());
    }

    #[test]
    fn test_revert_blocker_variants() {
        // Test that all blocker variants can be created
        let blocker1 = RevertBlocker::FileModifiedExternally {
            path: PathBuf::from("/test"),
            expected_hash: "expected".to_string(),
            current_hash: "current".to_string(),
        };
        assert!(matches!(
            blocker1,
            RevertBlocker::FileModifiedExternally { .. }
        ));

        let blocker2 = RevertBlocker::DependentActionExists {
            action_id: ActionId::from_string("dep"),
            description: "Test dependency".to_string(),
        };
        assert!(matches!(
            blocker2,
            RevertBlocker::DependentActionExists { .. }
        ));

        let blocker3 = RevertBlocker::ContentMissing {
            hash: "missing_hash".to_string(),
        };
        assert!(matches!(blocker3, RevertBlocker::ContentMissing { .. }));

        let blocker4 = RevertBlocker::NotRevertible {
            reason: "Cannot revert".to_string(),
        };
        assert!(matches!(blocker4, RevertBlocker::NotRevertible { .. }));
    }

    #[test]
    fn test_revert_warning_variants() {
        // Test that all warning variants can be created
        let warning1 = RevertWarning::SubsequentActionsExist {
            actions: vec![ActionId::from_string("a1"), ActionId::from_string("a2")],
        };
        assert!(matches!(
            warning1,
            RevertWarning::SubsequentActionsExist { .. }
        ));

        let warning2 = RevertWarning::WillInvalidateAction {
            action_id: ActionId::from_string("inv"),
            reason: "Will be invalidated".to_string(),
        };
        assert!(matches!(
            warning2,
            RevertWarning::WillInvalidateAction { .. }
        ));
    }
}
