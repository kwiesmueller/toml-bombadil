//! Persistent storage for sessions and action content.
//!
//! Storage layout:
//! ```text
//! .dots/
//! ├── audit/
//! │   ├── sessions/          # Session TOML files
//! │   ├── objects/           # Content-addressable storage (via ObjectStore)
//! │   ├── diffs/             # Human-readable unified diffs
//! │   ├── logs/              # Captured tracing output per action
//! │   └── index/
//! │       ├── actions.toml   # Action ID -> Session lookup
//! │       ├── files.toml     # File path -> [action_ids]
//! │       └── dag.toml       # Session parent/child relationships
//! ```

use super::action::ActionId;
use super::file_index::{FileAction, FileIndex};
use super::objects::ObjectStore;
use super::session::{Session, SessionDag, SessionId};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Index mapping action IDs to their sessions.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ActionIndex {
    /// Map from action ID to session ID.
    pub actions: HashMap<String, String>,
}

impl ActionIndex {
    /// Load or create the index.
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)
                .with_context(|| format!("Failed to read action index: {}", path.display()))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse action index: {}", path.display()))
        } else {
            Ok(Self::default())
        }
    }

    /// Save the index.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content =
            toml::to_string_pretty(self).context("Failed to serialize action index")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create index directory: {}", parent.display())
            })?;
        }
        fs::write(path, content)
            .with_context(|| format!("Failed to write action index: {}", path.display()))?;
        Ok(())
    }

    /// Add an action to the index.
    pub fn add(&mut self, action_id: &ActionId, session_id: &SessionId) {
        self.actions.insert(
            action_id.as_str().to_string(),
            session_id.as_str().to_string(),
        );
    }

    /// Look up which session contains an action.
    pub fn find_session(&self, action_id: &ActionId) -> Option<SessionId> {
        self.actions
            .get(action_id.as_str())
            .map(|s| SessionId::from_string(s.clone()))
    }
}

/// Storage manager for audit data.
pub struct AuditStorage {
    /// Base path for storage (typically dotfiles_dir/.dots/audit).
    base_path: PathBuf,
    /// Path to sessions directory.
    sessions_path: PathBuf,
    /// Content-addressable object store.
    objects: ObjectStore,
    /// Path to diffs directory.
    diffs_path: PathBuf,
    /// Path to logs directory.
    logs_path: PathBuf,
    /// Path to index directory.
    index_path: PathBuf,
    /// Legacy content path for backward compatibility.
    legacy_content_path: PathBuf,
}

impl AuditStorage {
    /// Create a new storage manager.
    pub fn new(dotfiles_dir: &Path) -> Self {
        let base_path = dotfiles_dir.join(".dots").join("audit");
        Self {
            sessions_path: base_path.join("sessions"),
            objects: ObjectStore::new(dotfiles_dir),
            diffs_path: base_path.join("diffs"),
            logs_path: base_path.join("logs"),
            index_path: base_path.join("index"),
            base_path,
            // Legacy path for backward compatibility
            legacy_content_path: dotfiles_dir.join(".dots").join("actions").join("content"),
        }
    }

    /// Initialize storage directories.
    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.sessions_path).with_context(|| {
            format!(
                "Failed to create sessions directory: {}",
                self.sessions_path.display()
            )
        })?;
        self.objects.init()?;
        fs::create_dir_all(&self.diffs_path).with_context(|| {
            format!(
                "Failed to create diffs directory: {}",
                self.diffs_path.display()
            )
        })?;
        fs::create_dir_all(&self.logs_path).with_context(|| {
            format!(
                "Failed to create logs directory: {}",
                self.logs_path.display()
            )
        })?;
        fs::create_dir_all(&self.index_path).with_context(|| {
            format!(
                "Failed to create index directory: {}",
                self.index_path.display()
            )
        })?;
        debug!(path = ?self.base_path, "Initialized audit storage");
        Ok(())
    }

    /// Get the path to the action index file.
    fn action_index_path(&self) -> PathBuf {
        self.index_path.join("actions.toml")
    }

    /// Get the path to the file index file.
    fn file_index_path(&self) -> PathBuf {
        self.index_path.join("files.toml")
    }

    /// Get the path to the session DAG file.
    fn dag_path(&self) -> PathBuf {
        self.index_path.join("dag.toml")
    }

    /// Store content using the object store (returns hash).
    pub fn store_content(&self, content: &[u8]) -> Result<String> {
        self.objects.store(content)
    }

    /// Load content by hash from the object store.
    pub fn load_content(&self, hash: &str) -> Result<Vec<u8>> {
        self.objects.load(hash)
    }

    /// Check if content exists in the object store.
    pub fn content_exists(&self, hash: &str) -> bool {
        self.objects.exists(hash)
    }

    /// Load the file index.
    pub fn load_file_index(&self) -> Result<FileIndex> {
        FileIndex::load(&self.file_index_path())
    }

    /// Save the file index.
    pub fn save_file_index(&self, index: &FileIndex) -> Result<()> {
        index.save(&self.file_index_path())
    }

    /// Load the session DAG.
    pub fn load_session_dag(&self) -> Result<SessionDag> {
        SessionDag::load(&self.dag_path())
    }

    /// Save the session DAG.
    pub fn save_session_dag(&self, dag: &SessionDag) -> Result<()> {
        dag.save(&self.dag_path())
    }

    /// Save an action diff.
    pub fn save_diff(&self, action_id: &ActionId, diff: &str) -> Result<()> {
        let path = self.diffs_path.join(format!("{}.diff", action_id));
        fs::write(&path, diff)
            .with_context(|| format!("Failed to write diff: {}", path.display()))?;
        debug!(action_id = %action_id, "Saved diff");
        Ok(())
    }

    /// Load an action diff.
    pub fn load_diff(&self, action_id: &ActionId) -> Result<String> {
        let path = self.diffs_path.join(format!("{}.diff", action_id));
        fs::read_to_string(&path)
            .with_context(|| format!("Failed to read diff: {}", path.display()))
    }

    /// Check if a diff exists for an action.
    pub fn has_diff(&self, action_id: &ActionId) -> bool {
        self.diffs_path
            .join(format!("{}.diff", action_id))
            .exists()
    }

    /// Save action logs.
    pub fn save_logs(&self, action_id: &ActionId, logs: &str) -> Result<()> {
        let path = self.logs_path.join(format!("{}.log", action_id));
        fs::write(&path, logs)
            .with_context(|| format!("Failed to write logs: {}", path.display()))?;
        debug!(action_id = %action_id, "Saved logs");
        Ok(())
    }

    /// Load action logs.
    pub fn load_logs(&self, action_id: &ActionId) -> Result<String> {
        let path = self.logs_path.join(format!("{}.log", action_id));
        fs::read_to_string(&path)
            .with_context(|| format!("Failed to read logs: {}", path.display()))
    }

    /// Check if logs exist for an action.
    pub fn has_logs(&self, action_id: &ActionId) -> bool {
        self.logs_path.join(format!("{}.log", action_id)).exists()
    }

    /// Get all actions for a file (from file index).
    pub fn actions_for_file(&self, path: &Path) -> Result<Vec<FileAction>> {
        let index = self.load_file_index()?;
        Ok(index.get(path).cloned().unwrap_or_default())
    }

    /// Save a session.
    pub fn save_session(&self, session: &Session) -> Result<()> {
        let filename = format!("{}.toml", session.id);
        let path = self.sessions_path.join(&filename);

        let content =
            toml::to_string_pretty(session).context("Failed to serialize session")?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write session: {}", path.display()))?;

        // Update the action index
        let mut index = ActionIndex::load(&self.action_index_path())?;
        for action in &session.actions {
            index.add(&action.id, &session.id);
        }
        index.save(&self.action_index_path())?;

        // Update the session DAG if session has a parent
        if let Some(ref parent_id) = session.parent_id {
            let mut dag = self.load_session_dag()?;
            dag.add(&session.id, parent_id);
            self.save_session_dag(&dag)?;
        }

        debug!(session_id = %session.id, actions = session.actions.len(), "Saved session");
        Ok(())
    }

    /// Load a session by ID.
    pub fn load_session(&self, id: &SessionId) -> Result<Session> {
        let filename = format!("{}.toml", id);
        let path = self.sessions_path.join(&filename);

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse session: {}", path.display()))
    }

    /// List all sessions, most recent first.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();

        if !self.sessions_path.exists() {
            return Ok(sessions);
        }

        for entry in fs::read_dir(&self.sessions_path).with_context(|| {
            format!(
                "Failed to read sessions directory: {}",
                self.sessions_path.display()
            )
        })? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "toml").unwrap_or(false) {
                match fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<Session>(&content) {
                        Ok(session) => sessions.push(session),
                        Err(e) => warn!(path = ?path, error = %e, "Failed to parse session"),
                    },
                    Err(e) => warn!(path = ?path, error = %e, "Failed to read session"),
                }
            }
        }

        // Sort by start time, most recent first
        sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(sessions)
    }

    /// Find a session containing an action.
    pub fn find_session_for_action(&self, action_id: &ActionId) -> Result<Option<Session>> {
        let index = ActionIndex::load(&self.action_index_path())?;
        if let Some(session_id) = index.find_session(action_id) {
            Ok(Some(self.load_session(&session_id)?))
        } else {
            Ok(None)
        }
    }

    /// Save content snapshot for an action (before state).
    ///
    /// Uses the object store internally but returns the path for compatibility.
    pub fn save_before_content(&self, action_id: &ActionId, content: &[u8]) -> Result<PathBuf> {
        let hash = self.objects.store(content)?;
        // Store a mapping file that points to the hash
        let mapping_path = self.index_path.join(format!("{}_before", action_id));
        fs::write(&mapping_path, &hash).with_context(|| {
            format!(
                "Failed to write before content mapping: {}",
                mapping_path.display()
            )
        })?;
        debug!(action_id = %action_id, hash = %hash, "Saved before content to object store");
        Ok(mapping_path)
    }

    /// Save content snapshot for an action (after state).
    ///
    /// Uses the object store internally but returns the path for compatibility.
    pub fn save_after_content(&self, action_id: &ActionId, content: &[u8]) -> Result<PathBuf> {
        let hash = self.objects.store(content)?;
        // Store a mapping file that points to the hash
        let mapping_path = self.index_path.join(format!("{}_after", action_id));
        fs::write(&mapping_path, &hash).with_context(|| {
            format!(
                "Failed to write after content mapping: {}",
                mapping_path.display()
            )
        })?;
        debug!(action_id = %action_id, hash = %hash, "Saved after content to object store");
        Ok(mapping_path)
    }

    /// Load before content for an action.
    ///
    /// Supports both new object store format and legacy direct content storage.
    pub fn load_before_content(&self, action_id: &ActionId) -> Result<Vec<u8>> {
        let mapping_path = self.index_path.join(format!("{}_before", action_id));

        // Try new format first (mapping file pointing to object store)
        if mapping_path.exists() {
            let hash = fs::read_to_string(&mapping_path).with_context(|| {
                format!(
                    "Failed to read before content mapping: {}",
                    mapping_path.display()
                )
            })?;
            return self.objects.load(hash.trim());
        }

        // Fall back to legacy format (direct content in .dots/actions/content/)
        let legacy_path = self.legacy_content_path.join(format!("{}_before", action_id));
        if legacy_path.exists() {
            return fs::read(&legacy_path).with_context(|| {
                format!(
                    "Failed to read legacy before content: {}",
                    legacy_path.display()
                )
            });
        }

        anyhow::bail!(
            "Before content not found for action {} (checked both new and legacy paths)",
            action_id
        )
    }

    /// Load after content for an action.
    ///
    /// Supports both new object store format and legacy direct content storage.
    pub fn load_after_content(&self, action_id: &ActionId) -> Result<Vec<u8>> {
        let mapping_path = self.index_path.join(format!("{}_after", action_id));

        // Try new format first (mapping file pointing to object store)
        if mapping_path.exists() {
            let hash = fs::read_to_string(&mapping_path).with_context(|| {
                format!(
                    "Failed to read after content mapping: {}",
                    mapping_path.display()
                )
            })?;
            return self.objects.load(hash.trim());
        }

        // Fall back to legacy format (direct content in .dots/actions/content/)
        let legacy_path = self.legacy_content_path.join(format!("{}_after", action_id));
        if legacy_path.exists() {
            return fs::read(&legacy_path).with_context(|| {
                format!(
                    "Failed to read legacy after content: {}",
                    legacy_path.display()
                )
            });
        }

        anyhow::bail!(
            "After content not found for action {} (checked both new and legacy paths)",
            action_id
        )
    }

    /// Check if content exists for an action.
    pub fn has_content(&self, action_id: &ActionId) -> bool {
        // Check new format
        let before_mapping = self.index_path.join(format!("{}_before", action_id));
        let after_mapping = self.index_path.join(format!("{}_after", action_id));
        if before_mapping.exists() || after_mapping.exists() {
            return true;
        }

        // Check legacy format
        let legacy_before = self.legacy_content_path.join(format!("{}_before", action_id));
        let legacy_after = self.legacy_content_path.join(format!("{}_after", action_id));
        legacy_before.exists() || legacy_after.exists()
    }

    /// Clean up old sessions, keeping only the most recent N.
    pub fn cleanup(&self, keep_count: usize) -> Result<usize> {
        let sessions = self.list_sessions()?;
        let mut removed = 0;

        if sessions.len() <= keep_count {
            return Ok(0);
        }

        // Get action IDs to remove
        let sessions_to_remove = &sessions[keep_count..];
        let mut actions_to_remove = Vec::new();

        for session in sessions_to_remove {
            // Remove session file
            let filename = format!("{}.toml", session.id);
            let path = self.sessions_path.join(&filename);
            if path.exists() {
                fs::remove_file(&path)?;
                removed += 1;
            }

            // Collect action IDs for content removal
            for action in &session.actions {
                actions_to_remove.push(action.id.clone());
            }
        }

        // Remove content mapping files, diffs, and logs
        for action_id in &actions_to_remove {
            // Remove mapping files
            let before_mapping = self.index_path.join(format!("{}_before", action_id));
            let after_mapping = self.index_path.join(format!("{}_after", action_id));
            let _ = fs::remove_file(before_mapping);
            let _ = fs::remove_file(after_mapping);

            // Remove diffs
            let diff_path = self.diffs_path.join(format!("{}.diff", action_id));
            let _ = fs::remove_file(diff_path);

            // Remove logs
            let log_path = self.logs_path.join(format!("{}.log", action_id));
            let _ = fs::remove_file(log_path);

            // Legacy content cleanup
            let legacy_before = self.legacy_content_path.join(format!("{}_before", action_id));
            let legacy_after = self.legacy_content_path.join(format!("{}_after", action_id));
            let _ = fs::remove_file(legacy_before);
            let _ = fs::remove_file(legacy_after);
        }

        // Update action index
        let mut index = ActionIndex::load(&self.action_index_path())?;
        for action_id in &actions_to_remove {
            index.actions.remove(action_id.as_str());
        }
        index.save(&self.action_index_path())?;

        // Note: We don't garbage collect the object store here because
        // other actions might reference the same content (deduplication).
        // A separate gc command could be added for that purpose.

        debug!(removed_sessions = removed, removed_actions = actions_to_remove.len(), "Cleaned up old sessions");
        Ok(removed)
    }

    /// Get total storage size in bytes.
    pub fn storage_size(&self) -> Result<u64> {
        fn dir_size(path: &Path) -> std::io::Result<u64> {
            let mut size = 0;
            if path.is_dir() {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    let metadata = entry.metadata()?;
                    if metadata.is_file() {
                        size += metadata.len();
                    } else if metadata.is_dir() {
                        size += dir_size(&entry.path())?;
                    }
                }
            }
            Ok(size)
        }

        dir_size(&self.base_path).context("Failed to calculate storage size")
    }

    /// Get the base path for storage.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Get the sessions path.
    pub fn sessions_path(&self) -> &Path {
        &self.sessions_path
    }

    /// Get a reference to the object store.
    pub fn objects(&self) -> &ObjectStore {
        &self.objects
    }
}

/// Compute SHA-256 hash of content.
pub fn content_hash(content: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::file_index::FileActionType;
    use crate::audit::session::Session;
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn test_storage_init() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        assert!(storage.sessions_path.exists());
        assert!(storage.diffs_path.exists());
        assert!(storage.logs_path.exists());
        assert!(storage.index_path.exists());
        // Object store creates its own directory
        assert!(dir.path().join(".dots").join("audit").join("objects").exists());
    }

    #[test]
    fn test_content_save_load() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let action_id = ActionId::from_string("test1");
        let content = b"Hello, World!";

        storage.save_before_content(&action_id, content).unwrap();
        let loaded = storage.load_before_content(&action_id).unwrap();

        assert_eq!(loaded, content);
    }

    #[test]
    fn test_content_save_load_after() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let action_id = ActionId::from_string("test2");
        let content = b"After content";

        storage.save_after_content(&action_id, content).unwrap();
        let loaded = storage.load_after_content(&action_id).unwrap();

        assert_eq!(loaded, content);
    }

    #[test]
    fn test_content_deduplication() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let content = b"Same content";
        let action_id1 = ActionId::from_string("test1");
        let action_id2 = ActionId::from_string("test2");

        // Save same content for two different actions
        storage.save_before_content(&action_id1, content).unwrap();
        storage.save_before_content(&action_id2, content).unwrap();

        // Both should load correctly
        let loaded1 = storage.load_before_content(&action_id1).unwrap();
        let loaded2 = storage.load_before_content(&action_id2).unwrap();
        assert_eq!(loaded1, content);
        assert_eq!(loaded2, content);

        // Content should be deduplicated (only one file in objects)
        let objects_path = dir.path().join(".dots").join("audit").join("objects");
        let mut file_count = 0;
        for entry in walkdir::WalkDir::new(&objects_path) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                file_count += 1;
            }
        }
        assert_eq!(file_count, 1, "Content should be deduplicated");
    }

    #[test]
    fn test_action_index() {
        let dir = tempdir().unwrap();
        let index_path = dir.path().join("index.toml");

        let mut index = ActionIndex::default();
        let action_id = ActionId::from_string("abc12");
        let session_id = SessionId::from_string("2024-01-01_xyz");

        index.add(&action_id, &session_id);
        index.save(&index_path).unwrap();

        let loaded = ActionIndex::load(&index_path).unwrap();
        assert_eq!(
            loaded.find_session(&action_id).unwrap().as_str(),
            session_id.as_str()
        );
    }

    #[test]
    fn test_diff_save_load() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let action_id = ActionId::from_string("diff1");
        let diff = "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new\n";

        storage.save_diff(&action_id, diff).unwrap();
        assert!(storage.has_diff(&action_id));

        let loaded = storage.load_diff(&action_id).unwrap();
        assert_eq!(loaded, diff);
    }

    #[test]
    fn test_logs_save_load() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let action_id = ActionId::from_string("log1");
        let logs = "2024-01-01T10:00:00Z INFO Starting action\n2024-01-01T10:00:01Z INFO Completed";

        storage.save_logs(&action_id, logs).unwrap();
        assert!(storage.has_logs(&action_id));

        let loaded = storage.load_logs(&action_id).unwrap();
        assert_eq!(loaded, logs);
    }

    #[test]
    fn test_file_index_save_load() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let mut index = FileIndex::new();
        let file_path = PathBuf::from("/home/user/.bashrc");
        let action = FileAction {
            action_id: ActionId::from_string("act1"),
            session_id: SessionId::from_string("sess1"),
            timestamp: Utc::now(),
            action_type: FileActionType::Create,
            content_hash: Some("hash123".to_string()),
        };
        index.add(file_path.clone(), action);

        storage.save_file_index(&index).unwrap();

        let loaded = storage.load_file_index().unwrap();
        assert!(loaded.get(&file_path).is_some());
        assert_eq!(loaded.get(&file_path).unwrap().len(), 1);
    }

    #[test]
    fn test_session_dag_save_load() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let mut dag = SessionDag::new();
        let parent = SessionId::from_string("parent1");
        let child = SessionId::from_string("child1");
        dag.add(&child, &parent);

        storage.save_session_dag(&dag).unwrap();

        let loaded = storage.load_session_dag().unwrap();
        assert_eq!(loaded.parent(&child).unwrap().as_str(), "parent1");
    }

    #[test]
    fn test_actions_for_file() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let mut index = FileIndex::new();
        let file_path = PathBuf::from("/home/user/.vimrc");

        let action1 = FileAction {
            action_id: ActionId::from_string("act1"),
            session_id: SessionId::from_string("sess1"),
            timestamp: Utc::now(),
            action_type: FileActionType::Create,
            content_hash: Some("hash1".to_string()),
        };
        let action2 = FileAction {
            action_id: ActionId::from_string("act2"),
            session_id: SessionId::from_string("sess2"),
            timestamp: Utc::now(),
            action_type: FileActionType::Update,
            content_hash: Some("hash2".to_string()),
        };

        index.add(file_path.clone(), action1);
        index.add(file_path.clone(), action2);
        storage.save_file_index(&index).unwrap();

        let actions = storage.actions_for_file(&file_path).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id.as_str(), "act1");
        assert_eq!(actions[1].action_id.as_str(), "act2");
    }

    #[test]
    fn test_has_content() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let action_id = ActionId::from_string("content1");
        assert!(!storage.has_content(&action_id));

        storage.save_before_content(&action_id, b"test").unwrap();
        assert!(storage.has_content(&action_id));
    }

    #[test]
    fn test_store_and_load_content_directly() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let content = b"Direct content storage test";
        let hash = storage.store_content(content).unwrap();

        assert!(storage.content_exists(&hash));
        let loaded = storage.load_content(&hash).unwrap();
        assert_eq!(loaded, content);
    }

    #[test]
    fn test_session_with_parent_updates_dag() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());
        storage.init().unwrap();

        let parent_id = SessionId::from_string("parent-session");
        let mut session = Session::new("link", vec!["default".to_string()]);
        session.set_parent(parent_id.clone());

        storage.save_session(&session).unwrap();

        let dag = storage.load_session_dag().unwrap();
        let loaded_parent = dag.parent(&session.id);
        assert!(loaded_parent.is_some());
        assert_eq!(loaded_parent.unwrap().as_str(), parent_id.as_str());
    }

    #[test]
    fn test_storage_paths() {
        let dir = tempdir().unwrap();
        let storage = AuditStorage::new(dir.path());

        assert_eq!(
            storage.base_path(),
            dir.path().join(".dots").join("audit")
        );
        assert_eq!(
            storage.sessions_path(),
            dir.path().join(".dots").join("audit").join("sessions")
        );
    }
}
