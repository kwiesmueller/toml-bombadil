//! Action types for the auditing system.
//!
//! Every change bombadil makes is tracked as an Action with a unique short hash ID.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Reference to a file with its content hash
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    pub path: PathBuf,
    /// SHA-256 of content
    pub content_hash: String,
    /// Whether file existed
    pub exists: bool,
}

impl FileRef {
    pub fn new(path: PathBuf, content_hash: String, exists: bool) -> Self {
        Self {
            path,
            content_hash,
            exists,
        }
    }

    /// Create from a path, computing hash if file exists
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        if path.exists() {
            let content = fs::read(path)?;
            let mut hasher = Sha256::new();
            hasher.update(&content);
            let hash = format!("{:x}", hasher.finalize());
            Ok(Self {
                path: path.to_path_buf(),
                content_hash: hash,
                exists: true,
            })
        } else {
            Ok(Self {
                path: path.to_path_buf(),
                content_hash: String::new(),
                exists: false,
            })
        }
    }
}

/// A unique 5-character identifier for an action.
///
/// Uses Crockford Base32 encoding (no I, L, O, U to avoid confusion).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionId(String);

impl ActionId {
    /// Generate a new ActionId from timestamp, action type, and target path.
    pub fn generate(timestamp: &DateTime<Utc>, action_type: &str, target: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        timestamp.timestamp_millis().hash(&mut hasher);
        action_type.hash(&mut hasher);
        target.hash(&mut hasher);
        let hash = hasher.finish();

        // Crockford Base32 alphabet (no I, L, O, U)
        const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

        let mut id = String::with_capacity(5);
        let mut value = hash;
        for _ in 0..5 {
            id.push(ALPHABET[(value % 32) as usize] as char);
            value /= 32;
        }

        Self(id.to_lowercase())
    }

    /// Create an ActionId from an existing string (for loading from storage).
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Get the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The type of action performed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ActionType {
    /// Created a new file from a dotfile source.
    FileCreate {
        target: PathBuf,
        source: PathBuf,
        content_hash: String,
    },

    /// Updated an existing file.
    FileUpdate {
        target: PathBuf,
        source: PathBuf,
        before_hash: String,
        after_hash: String,
    },

    /// Applied line-based patches to a file.
    FilePatch {
        target: PathBuf,
        base: PathBuf,
        patches_applied: Vec<String>,
        before_hash: String,
        after_hash: String,
    },

    /// Applied semantic patches to a structured file (JSON, YAML, TOML, INI).
    SemanticPatch {
        target: PathBuf,
        format: String,
        operations: Vec<String>,
        before_hash: String,
        after_hash: String,
    },

    /// Injected content into a file using markers.
    Inject {
        target: PathBuf,
        marker: String,
        before_hash: String,
        after_hash: String,
    },

    /// Created a symlink.
    SymlinkCreate { source: PathBuf, target: PathBuf },

    /// Removed a symlink.
    SymlinkRemove {
        target: PathBuf,
        was_pointing_to: PathBuf,
    },

    /// Backed up a file before overwriting.
    Backup {
        original: PathBuf,
        backup_location: PathBuf,
        content_hash: String,
    },

    /// Resolved a conflict between dotfile and system file.
    ConflictResolved {
        target: PathBuf,
        resolution: ConflictResolution,
        dotfile_hash: String,
        system_hash: String,
        /// Where the system file was backed up (set for UseDotfile / BackupAndReplace / Merged).
        /// Required to revert: restoring from this location undoes the conflict resolution.
        #[serde(default)]
        backup_location: Option<PathBuf>,
        /// Path to the dotfile source file in the repo (set for UseSystem / KeepSystem).
        #[serde(default)]
        source_path: Option<PathBuf>,
        /// Hash of the dotfile source content before it was overwritten with the system version.
        /// Content is stored in the object store under the action ID for revert.
        #[serde(default)]
        source_before_hash: Option<String>,
    },

    /// Executed a hook command.
    HookExecuted {
        command: String,
        hook_type: HookType,
        exit_code: i32,
    },
}

impl ActionType {
    /// Get a short description for display purposes.
    pub fn description(&self) -> String {
        match self {
            ActionType::FileCreate { target, .. } => {
                format!("Create file {}", target.display())
            }
            ActionType::FileUpdate { target, .. } => {
                format!("Update file {}", target.display())
            }
            ActionType::FilePatch {
                target,
                patches_applied,
                ..
            } => {
                format!(
                    "Patch {} ({} patches)",
                    target.display(),
                    patches_applied.len()
                )
            }
            ActionType::SemanticPatch { target, format, .. } => {
                format!("Semantic patch {} ({})", target.display(), format)
            }
            ActionType::Inject { target, marker, .. } => {
                format!("Inject into {} at '{}'", target.display(), marker)
            }
            ActionType::SymlinkCreate { target, .. } => {
                format!("Create symlink {}", target.display())
            }
            ActionType::SymlinkRemove { target, .. } => {
                format!("Remove symlink {}", target.display())
            }
            ActionType::Backup {
                original,
                backup_location,
                ..
            } => {
                format!(
                    "Backup {} to {}",
                    original.display(),
                    backup_location.display()
                )
            }
            ActionType::ConflictResolved {
                target, resolution, ..
            } => {
                format!("Resolve conflict for {} ({})", target.display(), resolution)
            }
            ActionType::HookExecuted {
                command,
                hook_type,
                exit_code,
            } => {
                format!("Run {} hook '{}' (exit {})", hook_type, command, exit_code)
            }
        }
    }

    /// Get the target path if this action affects a file.
    pub fn target_path(&self) -> Option<&PathBuf> {
        match self {
            ActionType::FileCreate { target, .. }
            | ActionType::FileUpdate { target, .. }
            | ActionType::FilePatch { target, .. }
            | ActionType::SemanticPatch { target, .. }
            | ActionType::Inject { target, .. }
            | ActionType::SymlinkCreate { target, .. }
            | ActionType::SymlinkRemove { target, .. }
            | ActionType::ConflictResolved { target, .. } => Some(target),
            ActionType::Backup { original, .. } => Some(original),
            ActionType::HookExecuted { .. } => None,
        }
    }

    /// Get a short type indicator for display.
    pub fn type_indicator(&self) -> &'static str {
        match self {
            ActionType::FileCreate { .. } => "+",
            ActionType::FileUpdate { .. } => "~",
            ActionType::FilePatch { .. } => "P",
            ActionType::SemanticPatch { .. } => "S",
            ActionType::Inject { .. } => "I",
            ActionType::SymlinkCreate { .. } => "L",
            ActionType::SymlinkRemove { .. } => "U",
            ActionType::Backup { .. } => "B",
            ActionType::ConflictResolved { .. } => "C",
            ActionType::HookExecuted { .. } => "H",
        }
    }

    /// Check if this action can be reverted.
    pub fn is_revertible(&self) -> bool {
        match self {
            ActionType::FileCreate { .. }
            | ActionType::FileUpdate { .. }
            | ActionType::FilePatch { .. }
            | ActionType::SemanticPatch { .. }
            | ActionType::Inject { .. }
            | ActionType::SymlinkCreate { .. }
            | ActionType::SymlinkRemove { .. }
            | ActionType::Backup { .. } => true,
            ActionType::ConflictResolved {
                resolution,
                backup_location,
                source_before_hash,
                ..
            } => match resolution {
                ConflictResolution::UseDotfile
                | ConflictResolution::BackupAndReplace
                | ConflictResolution::Merged => backup_location.is_some(),
                ConflictResolution::UseSystem | ConflictResolution::KeepSystem => {
                    source_before_hash.is_some()
                }
                ConflictResolution::Pending => false,
            },
            ActionType::HookExecuted { .. } => false,
        }
    }
}

/// How a conflict was resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Used the dotfile version.
    UseDotfile,
    /// Kept the system version (copied system content to dotfile source).
    UseSystem,
    /// Kept the system version (legacy alias).
    KeepSystem,
    /// Merged both versions.
    Merged,
    /// Backed up system and used dotfile.
    BackupAndReplace,
    /// Pending resolution (for interactive mode planning).
    Pending,
}

impl fmt::Display for ConflictResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictResolution::UseDotfile => write!(f, "use dotfile"),
            ConflictResolution::UseSystem | ConflictResolution::KeepSystem => {
                write!(f, "use system")
            }
            ConflictResolution::Merged => write!(f, "merged"),
            ConflictResolution::BackupAndReplace => write!(f, "backup and replace"),
            ConflictResolution::Pending => write!(f, "pending"),
        }
    }
}

/// Type of hook that was executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookType {
    PreInstall,
    PostInstall,
}

impl fmt::Display for HookType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookType::PreInstall => write!(f, "pre-install"),
            HookType::PostInstall => write!(f, "post-install"),
        }
    }
}

/// A single tracked action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// Unique identifier for this action.
    pub id: ActionId,
    /// When this action occurred.
    pub timestamp: DateTime<Utc>,
    /// The type and details of the action.
    pub action_type: ActionType,
    /// Optional dot name this action is associated with.
    pub dot_name: Option<String>,
    /// Files read as input (for dependency tracking)
    #[serde(default)]
    pub inputs: Vec<FileRef>,
    /// Files written as output
    #[serde(default)]
    pub outputs: Vec<FileRef>,
    /// Whether logs were captured for this action
    #[serde(default)]
    pub log_captured: bool,
}

impl Action {
    /// Create a new action with auto-generated ID.
    pub fn new(action_type: ActionType, dot_name: Option<String>) -> Self {
        let timestamp = Utc::now();
        let target_str = action_type
            .target_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let id = ActionId::generate(&timestamp, action_type.type_indicator(), &target_str);

        Self {
            id,
            timestamp,
            action_type,
            dot_name,
            inputs: Vec::new(),
            outputs: Vec::new(),
            log_captured: false,
        }
    }

    /// Check if this action can be reverted.
    pub fn is_revertible(&self) -> bool {
        self.action_type.is_revertible()
    }

    /// Add an input file reference
    pub fn add_input(&mut self, file_ref: FileRef) {
        self.inputs.push(file_ref);
    }

    /// Add an output file reference
    pub fn add_output(&mut self, file_ref: FileRef) {
        self.outputs.push(file_ref);
    }

    /// Get all file paths affected by this action (inputs + outputs)
    pub fn affected_paths(&self) -> Vec<&PathBuf> {
        self.inputs
            .iter()
            .map(|f| &f.path)
            .chain(self.outputs.iter().map(|f| &f.path))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_id_generation() {
        let timestamp = Utc::now();
        let id1 = ActionId::generate(&timestamp, "~", "/home/user/.bashrc");
        let id2 = ActionId::generate(&timestamp, "~", "/home/user/.bashrc");

        // Same inputs should produce same ID
        assert_eq!(id1, id2);
        assert_eq!(id1.as_str().len(), 5);

        // Different inputs should produce different IDs
        let id3 = ActionId::generate(&timestamp, "+", "/home/user/.bashrc");
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_action_type_indicators() {
        let create = ActionType::FileCreate {
            target: PathBuf::from("/test"),
            source: PathBuf::from("/src"),
            content_hash: "abc".to_string(),
        };
        assert_eq!(create.type_indicator(), "+");

        let symlink = ActionType::SymlinkCreate {
            source: PathBuf::from("/src"),
            target: PathBuf::from("/target"),
        };
        assert_eq!(symlink.type_indicator(), "L");
    }

    #[test]
    fn test_action_revertibility() {
        let file_update = ActionType::FileUpdate {
            target: PathBuf::from("/test"),
            source: PathBuf::from("/src"),
            before_hash: "a".to_string(),
            after_hash: "b".to_string(),
        };
        assert!(file_update.is_revertible());

        let hook = ActionType::HookExecuted {
            command: "echo test".to_string(),
            hook_type: HookType::PostInstall,
            exit_code: 0,
        };
        assert!(!hook.is_revertible());
    }

    #[test]
    fn test_file_ref_new() {
        let file_ref = FileRef::new(PathBuf::from("/test/file.txt"), "abc123".to_string(), true);
        assert_eq!(file_ref.path, PathBuf::from("/test/file.txt"));
        assert_eq!(file_ref.content_hash, "abc123");
        assert!(file_ref.exists);
    }

    #[test]
    fn test_file_ref_from_path_nonexistent() {
        let file_ref = FileRef::from_path(Path::new("/nonexistent/file.txt")).unwrap();
        assert_eq!(file_ref.path, PathBuf::from("/nonexistent/file.txt"));
        assert_eq!(file_ref.content_hash, "");
        assert!(!file_ref.exists);
    }

    #[test]
    fn test_file_ref_from_path_existing() {
        use std::io::Write;
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        let mut file = fs::File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let file_ref = FileRef::from_path(&temp_file).unwrap();
        assert_eq!(file_ref.path, temp_file);
        assert!(file_ref.exists);
        // SHA-256 of "hello world"
        assert_eq!(
            file_ref.content_hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_action_inputs_outputs() {
        let action_type = ActionType::FileCreate {
            target: PathBuf::from("/test"),
            source: PathBuf::from("/src"),
            content_hash: "abc".to_string(),
        };
        let mut action = Action::new(action_type, Some("my_dot".to_string()));

        // Initial state
        assert!(action.inputs.is_empty());
        assert!(action.outputs.is_empty());
        assert!(!action.log_captured);

        // Add input
        let input_ref = FileRef::new(PathBuf::from("/input/file.txt"), "hash1".to_string(), true);
        action.add_input(input_ref);
        assert_eq!(action.inputs.len(), 1);
        assert_eq!(action.inputs[0].path, PathBuf::from("/input/file.txt"));

        // Add output
        let output_ref = FileRef::new(PathBuf::from("/output/file.txt"), "hash2".to_string(), true);
        action.add_output(output_ref);
        assert_eq!(action.outputs.len(), 1);
        assert_eq!(action.outputs[0].path, PathBuf::from("/output/file.txt"));
    }

    #[test]
    fn test_action_affected_paths() {
        let action_type = ActionType::FileCreate {
            target: PathBuf::from("/test"),
            source: PathBuf::from("/src"),
            content_hash: "abc".to_string(),
        };
        let mut action = Action::new(action_type, None);

        let input1 = FileRef::new(PathBuf::from("/a"), "h1".to_string(), true);
        let input2 = FileRef::new(PathBuf::from("/b"), "h2".to_string(), true);
        let output1 = FileRef::new(PathBuf::from("/c"), "h3".to_string(), true);

        action.add_input(input1);
        action.add_input(input2);
        action.add_output(output1);

        let affected = action.affected_paths();
        assert_eq!(affected.len(), 3);
        assert!(affected.contains(&&PathBuf::from("/a")));
        assert!(affected.contains(&&PathBuf::from("/b")));
        assert!(affected.contains(&&PathBuf::from("/c")));
    }

    #[test]
    fn test_action_serde_with_new_fields() {
        let action_type = ActionType::FileCreate {
            target: PathBuf::from("/test"),
            source: PathBuf::from("/src"),
            content_hash: "abc".to_string(),
        };
        let mut action = Action::new(action_type, Some("dot".to_string()));
        action.add_input(FileRef::new(PathBuf::from("/in"), "h1".to_string(), true));
        action.add_output(FileRef::new(PathBuf::from("/out"), "h2".to_string(), false));
        action.log_captured = true;

        // Serialize and deserialize
        let json = serde_json::to_string(&action).unwrap();
        let deserialized: Action = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.inputs.len(), 1);
        assert_eq!(deserialized.inputs[0].path, PathBuf::from("/in"));
        assert_eq!(deserialized.outputs.len(), 1);
        assert_eq!(deserialized.outputs[0].path, PathBuf::from("/out"));
        assert!(!deserialized.outputs[0].exists);
        assert!(deserialized.log_captured);
    }

    #[test]
    fn test_action_serde_defaults_for_missing_fields() {
        // Simulate deserializing an old Action that doesn't have the new fields
        let json = r#"{
            "id": "abc12",
            "timestamp": "2024-01-01T00:00:00Z",
            "action_type": {
                "type": "FileCreate",
                "target": "/test",
                "source": "/src",
                "content_hash": "abc"
            },
            "dot_name": null
        }"#;

        let action: Action = serde_json::from_str(json).unwrap();
        assert!(action.inputs.is_empty());
        assert!(action.outputs.is_empty());
        assert!(!action.log_captured);
    }
}
