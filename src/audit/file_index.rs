//! File-centric action index.
//!
//! Maps file paths to ordered lists of actions that affected them.

use super::action::ActionId;
use super::session::SessionId;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Type of file action for the index
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileActionType {
    Read,   // File was input to action
    Create, // File was created
    Update, // File was modified
    Delete, // File was removed
}

/// Reference to an action that affected a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAction {
    pub action_id: ActionId,
    pub session_id: SessionId,
    pub timestamp: DateTime<Utc>,
    pub action_type: FileActionType,
    pub content_hash: Option<String>, // Hash after action (None if deleted)
}

/// Index mapping file paths to their action history
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FileIndex {
    /// Map: canonical file path -> ordered list of action references
    pub files: HashMap<PathBuf, Vec<FileAction>>,
}

impl FileIndex {
    /// Create a new empty file index.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Load from disk.
    pub fn load(path: &Path) -> Result<Self> {
        debug!("Loading file index from {}", path.display());

        if !path.exists() {
            debug!("File index does not exist, returning empty index");
            return Ok(Self::new());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file index from {}", path.display()))?;

        let index: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse file index from {}", path.display()))?;

        debug!("Loaded file index with {} tracked files", index.files.len());
        Ok(index)
    }

    /// Save to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        debug!("Saving file index to {}", path.display());

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create directory for file index: {}",
                    parent.display()
                )
            })?;
        }

        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize file index to TOML")?;

        fs::write(path, content)
            .with_context(|| format!("Failed to write file index to {}", path.display()))?;

        debug!("Saved file index with {} tracked files", self.files.len());
        Ok(())
    }

    /// Add a file action to the index.
    ///
    /// Actions are stored in chronological order (oldest first, newest last).
    pub fn add(&mut self, path: PathBuf, action: FileAction) {
        debug!(
            "Adding {:?} action for {} (action_id: {})",
            action.action_type,
            path.display(),
            action.action_id
        );

        let actions = self.files.entry(path).or_default();
        actions.push(action);
    }

    /// Get all actions for a file path.
    pub fn get(&self, path: &Path) -> Option<&Vec<FileAction>> {
        self.files.get(path)
    }

    /// Get the most recent action for a file.
    pub fn latest(&self, path: &Path) -> Option<&FileAction> {
        self.files.get(path).and_then(|actions| actions.last())
    }

    /// Get actions newer than a given action ID for a file.
    ///
    /// Returns all actions that occurred after the action with the given ID.
    /// If the action ID is not found for this file, returns an empty vector.
    pub fn actions_after(&self, path: &Path, action_id: &ActionId) -> Vec<&FileAction> {
        match self.files.get(path) {
            Some(actions) => {
                // Find the index of the action with the given ID
                let start_idx = actions
                    .iter()
                    .position(|a| &a.action_id == action_id)
                    .map(|idx| idx + 1); // Start from the action after

                match start_idx {
                    Some(idx) => actions[idx..].iter().collect(),
                    None => Vec::new(),
                }
            }
            None => Vec::new(),
        }
    }

    /// List all tracked files.
    pub fn files(&self) -> Vec<&PathBuf> {
        self.files.keys().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    fn make_action(
        action_id: &str,
        session_id: &str,
        timestamp: DateTime<Utc>,
        action_type: FileActionType,
        content_hash: Option<&str>,
    ) -> FileAction {
        FileAction {
            action_id: ActionId::from_string(action_id),
            session_id: SessionId::from_string(session_id),
            timestamp,
            action_type,
            content_hash: content_hash.map(String::from),
        }
    }

    #[test]
    fn test_new_creates_empty_index() {
        let index = FileIndex::new();
        assert!(index.files.is_empty());
    }

    #[test]
    fn test_add_and_get() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.bashrc");
        let timestamp = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();

        let action = make_action(
            "abc12",
            "session_001",
            timestamp,
            FileActionType::Create,
            Some("hash123"),
        );

        index.add(path.clone(), action);

        let actions = index.get(&path);
        assert!(actions.is_some());
        let actions = actions.unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id.as_str(), "abc12");
        assert_eq!(actions[0].action_type, FileActionType::Create);
    }

    #[test]
    fn test_multiple_actions_same_file() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.bashrc");

        let t1 = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 1, 15, 11, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        index.add(
            path.clone(),
            make_action("act01", "s1", t1, FileActionType::Create, Some("h1")),
        );
        index.add(
            path.clone(),
            make_action("act02", "s2", t2, FileActionType::Update, Some("h2")),
        );
        index.add(
            path.clone(),
            make_action("act03", "s3", t3, FileActionType::Update, Some("h3")),
        );

        let actions = index.get(&path).unwrap();
        assert_eq!(actions.len(), 3);

        // Verify order (oldest first)
        assert_eq!(actions[0].action_id.as_str(), "act01");
        assert_eq!(actions[1].action_id.as_str(), "act02");
        assert_eq!(actions[2].action_id.as_str(), "act03");
    }

    #[test]
    fn test_ordering_newest_last() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.config/test");

        let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2024, 12, 1, 0, 0, 0).unwrap();

        // Add in order
        index.add(
            path.clone(),
            make_action("first", "s1", t1, FileActionType::Create, Some("h1")),
        );
        index.add(
            path.clone(),
            make_action("second", "s2", t2, FileActionType::Update, Some("h2")),
        );
        index.add(
            path.clone(),
            make_action("third", "s3", t3, FileActionType::Update, Some("h3")),
        );

        let actions = index.get(&path).unwrap();

        // Newest should be last
        assert_eq!(actions.last().unwrap().action_id.as_str(), "third");
        assert_eq!(actions.first().unwrap().action_id.as_str(), "first");
    }

    #[test]
    fn test_latest() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.vimrc");

        let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        index.add(
            path.clone(),
            make_action("old", "s1", t1, FileActionType::Create, Some("h1")),
        );
        index.add(
            path.clone(),
            make_action("new", "s2", t2, FileActionType::Update, Some("h2")),
        );

        let latest = index.latest(&path);
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().action_id.as_str(), "new");
    }

    #[test]
    fn test_latest_returns_none_for_unknown_file() {
        let index = FileIndex::new();
        let latest = index.latest(Path::new("/unknown/file"));
        assert!(latest.is_none());
    }

    #[test]
    fn test_actions_after() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.bashrc");

        let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap();
        let t4 = Utc.with_ymd_and_hms(2024, 4, 1, 0, 0, 0).unwrap();

        index.add(
            path.clone(),
            make_action("a1", "s1", t1, FileActionType::Create, Some("h1")),
        );
        index.add(
            path.clone(),
            make_action("a2", "s2", t2, FileActionType::Update, Some("h2")),
        );
        index.add(
            path.clone(),
            make_action("a3", "s3", t3, FileActionType::Update, Some("h3")),
        );
        index.add(
            path.clone(),
            make_action("a4", "s4", t4, FileActionType::Update, Some("h4")),
        );

        // Get actions after "a2"
        let after_a2 = index.actions_after(&path, &ActionId::from_string("a2"));
        assert_eq!(after_a2.len(), 2);
        assert_eq!(after_a2[0].action_id.as_str(), "a3");
        assert_eq!(after_a2[1].action_id.as_str(), "a4");

        // Get actions after the first action
        let after_a1 = index.actions_after(&path, &ActionId::from_string("a1"));
        assert_eq!(after_a1.len(), 3);

        // Get actions after the last action (should be empty)
        let after_a4 = index.actions_after(&path, &ActionId::from_string("a4"));
        assert!(after_a4.is_empty());
    }

    #[test]
    fn test_actions_after_unknown_action_id() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.bashrc");
        let timestamp = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        index.add(
            path.clone(),
            make_action("a1", "s1", timestamp, FileActionType::Create, Some("h1")),
        );

        // Unknown action ID should return empty
        let result = index.actions_after(&path, &ActionId::from_string("unknown"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_actions_after_unknown_file() {
        let index = FileIndex::new();
        let result = index.actions_after(Path::new("/unknown"), &ActionId::from_string("a1"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_files_list() {
        let mut index = FileIndex::new();
        let timestamp = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        let path1 = PathBuf::from("/home/user/.bashrc");
        let path2 = PathBuf::from("/home/user/.vimrc");
        let path3 = PathBuf::from("/home/user/.gitconfig");

        index.add(
            path1.clone(),
            make_action("a1", "s1", timestamp, FileActionType::Create, Some("h1")),
        );
        index.add(
            path2.clone(),
            make_action("a2", "s1", timestamp, FileActionType::Create, Some("h2")),
        );
        index.add(
            path3.clone(),
            make_action("a3", "s1", timestamp, FileActionType::Create, Some("h3")),
        );

        let files = index.files();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&&path1));
        assert!(files.contains(&&path2));
        assert!(files.contains(&&path3));
    }

    #[test]
    fn test_get_returns_none_for_unknown_file() {
        let index = FileIndex::new();
        assert!(index.get(Path::new("/unknown/path")).is_none());
    }

    #[test]
    fn test_delete_action_type() {
        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.bashrc");

        let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        index.add(
            path.clone(),
            make_action("a1", "s1", t1, FileActionType::Create, Some("h1")),
        );
        index.add(
            path.clone(),
            make_action("a2", "s2", t2, FileActionType::Delete, None),
        );

        let latest = index.latest(&path).unwrap();
        assert_eq!(latest.action_type, FileActionType::Delete);
        assert!(latest.content_hash.is_none());
    }

    #[test]
    fn test_save_and_load() -> Result<()> {
        let dir = tempdir()?;
        let index_path = dir.path().join("file_index.toml");

        let mut index = FileIndex::new();
        let path = PathBuf::from("/home/user/.bashrc");
        let timestamp = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();

        index.add(
            path.clone(),
            make_action("abc12", "session_001", timestamp, FileActionType::Create, Some("hash123")),
        );
        index.add(
            path.clone(),
            make_action("def34", "session_002", timestamp, FileActionType::Update, Some("hash456")),
        );

        // Save
        index.save(&index_path)?;

        // Verify file exists
        assert!(index_path.exists());

        // Load
        let loaded = FileIndex::load(&index_path)?;

        // Verify loaded data
        assert_eq!(loaded.files.len(), 1);
        let actions = loaded.get(&path).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id.as_str(), "abc12");
        assert_eq!(actions[1].action_id.as_str(), "def34");

        Ok(())
    }

    #[test]
    fn test_load_nonexistent_file() -> Result<()> {
        let index = FileIndex::load(Path::new("/nonexistent/path/index.toml"))?;
        assert!(index.files.is_empty());
        Ok(())
    }

    #[test]
    fn test_save_creates_parent_directories() -> Result<()> {
        let dir = tempdir()?;
        let nested_path = dir.path().join("nested").join("deep").join("file_index.toml");

        let index = FileIndex::new();
        index.save(&nested_path)?;

        assert!(nested_path.exists());
        Ok(())
    }
}
