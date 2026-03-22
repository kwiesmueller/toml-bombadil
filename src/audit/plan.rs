//! Action planning for dry-run and interactive modes.
//!
//! An ActionPlan represents a set of actions that will be performed,
//! allowing users to review and approve changes before execution.

use super::action::{Action, ActionType};
use std::path::PathBuf;

/// A planned action that hasn't been executed yet.
#[derive(Debug, Clone)]
pub struct PlannedAction {
    /// The action that will be performed.
    pub action_type: ActionType,
    /// Optional dot name this action is associated with.
    pub dot_name: Option<String>,
    /// Whether this action is approved for execution (in interactive mode).
    pub approved: bool,
    /// Preview of the diff if available.
    pub diff_preview: Option<String>,
}

impl PlannedAction {
    /// Create a new planned action.
    pub fn new(action_type: ActionType, dot_name: Option<String>) -> Self {
        Self {
            action_type,
            dot_name,
            approved: true, // Default to approved in non-interactive mode
            diff_preview: None,
        }
    }

    /// Create with a diff preview.
    pub fn with_diff(mut self, diff: String) -> Self {
        self.diff_preview = Some(diff);
        self
    }

    /// Convert to an Action (for execution).
    pub fn into_action(self) -> Action {
        Action::new(self.action_type, self.dot_name)
    }
}

/// A plan of actions to be executed.
#[derive(Debug, Default)]
pub struct ActionPlan {
    /// All planned actions.
    actions: Vec<PlannedAction>,
}

impl ActionPlan {
    /// Create a new empty plan.
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    /// Add a planned action.
    pub fn add(&mut self, action: PlannedAction) {
        self.actions.push(action);
    }

    /// Add a planned action from action type.
    pub fn add_action(&mut self, action_type: ActionType, dot_name: Option<String>) {
        self.actions.push(PlannedAction::new(action_type, dot_name));
    }

    /// Get all planned actions.
    pub fn actions(&self) -> &[PlannedAction] {
        &self.actions
    }

    /// Get mutable access to planned actions (for interactive approval).
    pub fn actions_mut(&mut self) -> &mut [PlannedAction] {
        &mut self.actions
    }

    /// Get only approved actions.
    pub fn approved_actions(&self) -> impl Iterator<Item = &PlannedAction> {
        self.actions.iter().filter(|a| a.approved)
    }

    /// Approve all actions.
    pub fn approve_all(&mut self) {
        for action in &mut self.actions {
            action.approved = true;
        }
    }

    /// Reject all actions.
    pub fn reject_all(&mut self) {
        for action in &mut self.actions {
            action.approved = false;
        }
    }

    /// Approve a specific action by index.
    pub fn approve(&mut self, index: usize) -> bool {
        if let Some(action) = self.actions.get_mut(index) {
            action.approved = true;
            true
        } else {
            false
        }
    }

    /// Reject a specific action by index.
    pub fn reject(&mut self, index: usize) -> bool {
        if let Some(action) = self.actions.get_mut(index) {
            action.approved = false;
            true
        } else {
            false
        }
    }

    /// Check if the plan is empty.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Get the number of planned actions.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Get the number of approved actions.
    pub fn approved_count(&self) -> usize {
        self.actions.iter().filter(|a| a.approved).count()
    }

    /// Get actions grouped by target file.
    pub fn by_target(&self) -> std::collections::HashMap<PathBuf, Vec<&PlannedAction>> {
        let mut map = std::collections::HashMap::new();
        for action in &self.actions {
            if let Some(path) = action.action_type.target_path() {
                map.entry(path.clone())
                    .or_insert_with(Vec::new)
                    .push(action);
            }
        }
        map
    }

    /// Get actions grouped by dot name.
    pub fn by_dot(&self) -> std::collections::HashMap<String, Vec<&PlannedAction>> {
        let mut map = std::collections::HashMap::new();
        for action in &self.actions {
            if let Some(name) = &action.dot_name {
                map.entry(name.clone())
                    .or_insert_with(Vec::new)
                    .push(action);
            }
        }
        map
    }
}

/// Summary of what a plan will do.
#[derive(Debug, Default)]
pub struct PlanSummary {
    pub files_to_create: usize,
    pub files_to_update: usize,
    pub files_to_patch: usize,
    pub symlinks_to_create: usize,
    pub symlinks_to_remove: usize,
    pub conflicts_to_resolve: usize,
    pub hooks_to_run: usize,
}

impl PlanSummary {
    /// Generate a summary from a plan.
    pub fn from_plan(plan: &ActionPlan) -> Self {
        let mut summary = Self::default();

        for action in plan.approved_actions() {
            match &action.action_type {
                ActionType::FileCreate { .. } => summary.files_to_create += 1,
                ActionType::FileUpdate { .. } => summary.files_to_update += 1,
                ActionType::FilePatch { .. }
                | ActionType::SemanticPatch { .. }
                | ActionType::Inject { .. } => summary.files_to_patch += 1,
                ActionType::SymlinkCreate { .. } => summary.symlinks_to_create += 1,
                ActionType::SymlinkRemove { .. } => summary.symlinks_to_remove += 1,
                ActionType::ConflictResolved { .. } => summary.conflicts_to_resolve += 1,
                ActionType::HookExecuted { .. } => summary.hooks_to_run += 1,
                ActionType::Backup { .. } => {} // Backups don't count in summary
            }
        }

        summary
    }

    /// Check if there are any actions to perform.
    pub fn has_actions(&self) -> bool {
        self.files_to_create > 0
            || self.files_to_update > 0
            || self.files_to_patch > 0
            || self.symlinks_to_create > 0
            || self.symlinks_to_remove > 0
            || self.conflicts_to_resolve > 0
            || self.hooks_to_run > 0
    }

    /// Format a one-line summary.
    pub fn one_line(&self) -> String {
        let mut parts = Vec::new();

        if self.files_to_create > 0 {
            parts.push(format!("{} to create", self.files_to_create));
        }
        if self.files_to_update > 0 {
            parts.push(format!("{} to update", self.files_to_update));
        }
        if self.files_to_patch > 0 {
            parts.push(format!("{} to patch", self.files_to_patch));
        }
        if self.symlinks_to_create > 0 {
            parts.push(format!("{} symlinks", self.symlinks_to_create));
        }
        if self.conflicts_to_resolve > 0 {
            parts.push(format!("{} conflicts", self.conflicts_to_resolve));
        }

        if parts.is_empty() {
            "Nothing to do".to_string()
        } else {
            parts.join(", ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_creation() {
        let mut plan = ActionPlan::new();
        assert!(plan.is_empty());

        plan.add_action(
            ActionType::FileCreate {
                target: PathBuf::from("/test"),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            Some("test".to_string()),
        );

        assert_eq!(plan.len(), 1);
        assert_eq!(plan.approved_count(), 1);
    }

    #[test]
    fn test_plan_approval() {
        let mut plan = ActionPlan::new();
        plan.add_action(
            ActionType::FileCreate {
                target: PathBuf::from("/test"),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            None,
        );

        plan.reject(0);
        assert_eq!(plan.approved_count(), 0);

        plan.approve(0);
        assert_eq!(plan.approved_count(), 1);
    }

    #[test]
    fn test_plan_summary() {
        let mut plan = ActionPlan::new();
        plan.add_action(
            ActionType::FileCreate {
                target: PathBuf::from("/test1"),
                source: PathBuf::from("/src1"),
                content_hash: "abc".to_string(),
            },
            None,
        );
        plan.add_action(
            ActionType::FileUpdate {
                target: PathBuf::from("/test2"),
                source: PathBuf::from("/src2"),
                before_hash: "a".to_string(),
                after_hash: "b".to_string(),
            },
            None,
        );

        let summary = PlanSummary::from_plan(&plan);
        assert_eq!(summary.files_to_create, 1);
        assert_eq!(summary.files_to_update, 1);
        assert!(summary.has_actions());
    }
}
