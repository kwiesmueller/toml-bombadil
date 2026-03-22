//! Session management for tracking a group of actions.
//!
//! A session represents a single bombadil operation (e.g., `bombadil link`)
//! and contains all actions performed during that operation.

use super::action::{Action, ActionId, ActionType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A unique identifier for a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Generate a new session ID from timestamp and a short hash.
    pub fn generate(timestamp: &DateTime<Utc>) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        timestamp.timestamp_millis().hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        let hash = hasher.finish();

        // Use first 5 chars of hex hash
        let hash_suffix: String = format!("{:x}", hash).chars().take(5).collect();

        Self(format!(
            "{}_{}",
            timestamp.format("%Y-%m-%dT%H-%M-%S"),
            hash_suffix
        ))
    }

    /// Create a SessionId from an existing string.
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Get the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Statistics about a session's actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStats {
    pub files_created: usize,
    pub files_updated: usize,
    pub files_patched: usize,
    pub symlinks_created: usize,
    pub symlinks_removed: usize,
    pub conflicts_resolved: usize,
    pub backups_made: usize,
    pub hooks_executed: usize,
}

impl SessionStats {
    /// Update stats based on an action.
    pub fn record(&mut self, action: &Action) {
        match &action.action_type {
            ActionType::FileCreate { .. } => self.files_created += 1,
            ActionType::FileUpdate { .. } => self.files_updated += 1,
            ActionType::FilePatch { .. }
            | ActionType::SemanticPatch { .. }
            | ActionType::Inject { .. } => self.files_patched += 1,
            ActionType::SymlinkCreate { .. } => self.symlinks_created += 1,
            ActionType::SymlinkRemove { .. } => self.symlinks_removed += 1,
            ActionType::ConflictResolved { .. } => self.conflicts_resolved += 1,
            ActionType::Backup { .. } => self.backups_made += 1,
            ActionType::HookExecuted { .. } => self.hooks_executed += 1,
        }
    }

    /// Format a summary line for display.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if self.files_created > 0 {
            parts.push(format!(
                "{} file{} created",
                self.files_created,
                if self.files_created == 1 { "" } else { "s" }
            ));
        }
        if self.files_updated > 0 {
            parts.push(format!(
                "{} file{} updated",
                self.files_updated,
                if self.files_updated == 1 { "" } else { "s" }
            ));
        }
        if self.files_patched > 0 {
            parts.push(format!(
                "{} file{} patched",
                self.files_patched,
                if self.files_patched == 1 { "" } else { "s" }
            ));
        }
        if self.symlinks_created > 0 {
            parts.push(format!(
                "{} symlink{} created",
                self.symlinks_created,
                if self.symlinks_created == 1 { "" } else { "s" }
            ));
        }
        if self.symlinks_removed > 0 {
            parts.push(format!(
                "{} symlink{} removed",
                self.symlinks_removed,
                if self.symlinks_removed == 1 { "" } else { "s" }
            ));
        }
        if self.conflicts_resolved > 0 {
            parts.push(format!(
                "{} conflict{} resolved",
                self.conflicts_resolved,
                if self.conflicts_resolved == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }

        if parts.is_empty() {
            "No changes made".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// A session containing multiple actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique identifier for this session.
    pub id: SessionId,
    /// When this session started.
    pub started_at: DateTime<Utc>,
    /// When this session completed (if finished).
    pub completed_at: Option<DateTime<Utc>>,
    /// The command that initiated this session.
    pub command: String,
    /// Actions performed in this session.
    pub actions: Vec<Action>,
    /// Statistics about the session.
    pub stats: SessionStats,
    /// Profiles that were active during this session.
    pub profiles: Vec<String>,
    /// Parent session ID (for DAG tracking).
    #[serde(default)]
    pub parent_id: Option<SessionId>,
}

impl Session {
    /// Create a new session.
    pub fn new(command: impl Into<String>, profiles: Vec<String>) -> Self {
        let started_at = Utc::now();
        let id = SessionId::generate(&started_at);

        Self {
            id,
            started_at,
            completed_at: None,
            command: command.into(),
            actions: Vec::new(),
            stats: SessionStats::default(),
            profiles,
            parent_id: None,
        }
    }

    /// Set the parent session ID.
    pub fn set_parent(&mut self, parent_id: SessionId) {
        self.parent_id = Some(parent_id);
    }

    /// Check if this session has a parent.
    pub fn has_parent(&self) -> bool {
        self.parent_id.is_some()
    }

    /// Add an action to this session.
    pub fn add_action(&mut self, action: Action) {
        self.stats.record(&action);
        self.actions.push(action);
    }

    /// Mark the session as complete.
    pub fn complete(&mut self) {
        self.completed_at = Some(Utc::now());
    }

    /// Get an action by its ID.
    pub fn get_action(&self, id: &ActionId) -> Option<&Action> {
        self.actions.iter().find(|a| &a.id == id)
    }

    /// Get all actions affecting a specific file.
    pub fn actions_for_file(&self, path: &PathBuf) -> Vec<&Action> {
        self.actions
            .iter()
            .filter(|a| a.action_type.target_path() == Some(path))
            .collect()
    }

    /// Get all actions for a specific dot.
    pub fn actions_for_dot(&self, dot_name: &str) -> Vec<&Action> {
        self.actions
            .iter()
            .filter(|a| a.dot_name.as_deref() == Some(dot_name))
            .collect()
    }

    /// Check if session has any actions.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

/// DAG tracking session parent/child relationships.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionDag {
    /// Map: session_id -> parent session_id
    pub parents: HashMap<String, String>,
}

impl SessionDag {
    /// Create a new empty SessionDag.
    pub fn new() -> Self {
        Self {
            parents: HashMap::new(),
        }
    }

    /// Load from disk.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let dag: SessionDag = serde_json::from_str(&content)?;
            Ok(dag)
        } else {
            Ok(Self::new())
        }
    }

    /// Save to disk.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Record a parent-child relationship.
    pub fn add(&mut self, child: &SessionId, parent: &SessionId) {
        self.parents
            .insert(child.as_str().to_string(), parent.as_str().to_string());
    }

    /// Get parent of a session.
    pub fn parent(&self, session_id: &SessionId) -> Option<SessionId> {
        self.parents
            .get(session_id.as_str())
            .map(|s| SessionId::from_string(s.clone()))
    }

    /// Get children of a session.
    pub fn children(&self, parent_id: &SessionId) -> Vec<SessionId> {
        let parent_str = parent_id.as_str();
        self.parents
            .iter()
            .filter(|(_, p)| p.as_str() == parent_str)
            .map(|(c, _)| SessionId::from_string(c.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_session_creation() {
        let session = Session::new("link", vec!["default".to_string()]);
        assert!(session.actions.is_empty());
        assert!(session.completed_at.is_none());
        assert_eq!(session.profiles, vec!["default"]);
    }

    #[test]
    fn test_session_stats() {
        let mut session = Session::new("link", vec![]);

        let action = Action::new(
            ActionType::FileCreate {
                target: PathBuf::from("/test"),
                source: PathBuf::from("/src"),
                content_hash: "abc".to_string(),
            },
            Some("test-dot".to_string()),
        );
        session.add_action(action);

        assert_eq!(session.stats.files_created, 1);
        assert_eq!(session.stats.summary(), "1 file created");
    }

    #[test]
    fn test_session_id_format() {
        let timestamp = Utc::now();
        let id = SessionId::generate(&timestamp);
        let id_str = id.as_str();

        // Should contain date-time format and hash suffix
        assert!(id_str.contains("T"));
        assert!(id_str.contains("_"));
    }

    #[test]
    fn test_session_parent_id() {
        let mut session = Session::new("link", vec![]);
        assert!(!session.has_parent());
        assert!(session.parent_id.is_none());

        let parent_id = SessionId::from_string("parent-session-123");
        session.set_parent(parent_id.clone());

        assert!(session.has_parent());
        assert_eq!(session.parent_id.unwrap().as_str(), "parent-session-123");
    }

    #[test]
    fn test_session_dag_new() {
        let dag = SessionDag::new();
        assert!(dag.parents.is_empty());
    }

    #[test]
    fn test_session_dag_add_and_parent() {
        let mut dag = SessionDag::new();
        let parent = SessionId::from_string("parent-1");
        let child = SessionId::from_string("child-1");

        dag.add(&child, &parent);

        let found_parent = dag.parent(&child);
        assert!(found_parent.is_some());
        assert_eq!(found_parent.unwrap().as_str(), "parent-1");

        // Non-existent session should return None
        let orphan = SessionId::from_string("orphan");
        assert!(dag.parent(&orphan).is_none());
    }

    #[test]
    fn test_session_dag_children() {
        let mut dag = SessionDag::new();
        let parent = SessionId::from_string("parent-1");
        let child1 = SessionId::from_string("child-1");
        let child2 = SessionId::from_string("child-2");
        let child3 = SessionId::from_string("child-3");

        dag.add(&child1, &parent);
        dag.add(&child2, &parent);

        // child3 has a different parent
        let other_parent = SessionId::from_string("parent-2");
        dag.add(&child3, &other_parent);

        let children = dag.children(&parent);
        assert_eq!(children.len(), 2);

        let child_strs: Vec<&str> = children.iter().map(|c| c.as_str()).collect();
        assert!(child_strs.contains(&"child-1"));
        assert!(child_strs.contains(&"child-2"));
        assert!(!child_strs.contains(&"child-3"));
    }

    #[test]
    fn test_session_dag_save_and_load() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dag_path = temp_dir.path().join("dag.json");

        let mut dag = SessionDag::new();
        let parent = SessionId::from_string("parent-1");
        let child1 = SessionId::from_string("child-1");
        let child2 = SessionId::from_string("child-2");

        dag.add(&child1, &parent);
        dag.add(&child2, &parent);

        // Save
        dag.save(&dag_path).unwrap();

        // Load
        let loaded_dag = SessionDag::load(&dag_path).unwrap();
        assert_eq!(loaded_dag.parents.len(), 2);
        assert_eq!(loaded_dag.parent(&child1).unwrap().as_str(), "parent-1");
        assert_eq!(loaded_dag.parent(&child2).unwrap().as_str(), "parent-1");
    }

    #[test]
    fn test_session_dag_load_nonexistent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dag_path = temp_dir.path().join("nonexistent.json");

        // Loading a non-existent file should return an empty DAG
        let dag = SessionDag::load(&dag_path).unwrap();
        assert!(dag.parents.is_empty());
    }
}
