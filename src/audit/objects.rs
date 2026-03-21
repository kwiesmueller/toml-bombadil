//! Content-addressable object store (git-like).
//!
//! Stores content by SHA-256 hash for deduplication.
//! Storage path: objects/{hash[0..2]}/{hash[2..]}

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Content-addressable object store for storing file content by hash.
///
/// Objects are stored in a git-like directory structure where the first
/// two characters of the hash form a subdirectory, and the remaining
/// characters form the filename. This helps avoid having too many files
/// in a single directory.
pub struct ObjectStore {
    /// Base path for object storage (.dots/audit/objects/)
    base_path: PathBuf,
}

impl ObjectStore {
    /// Create a new ObjectStore with the given dotfiles directory.
    ///
    /// The actual storage will be at `dotfiles_dir/.dots/audit/objects/`.
    pub fn new(dotfiles_dir: &Path) -> Self {
        Self {
            base_path: dotfiles_dir.join(".dots").join("audit").join("objects"),
        }
    }

    /// Initialize storage directories.
    ///
    /// Creates the base objects directory if it doesn't exist.
    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.base_path)
            .with_context(|| format!("Failed to create object store at {:?}", self.base_path))?;
        debug!("Initialized object store at {:?}", self.base_path);
        Ok(())
    }

    /// Store content, returns SHA-256 hash (hex encoded).
    ///
    /// If the content already exists (same hash), this is a no-op
    /// and returns the existing hash without writing duplicate data.
    pub fn store(&self, content: &[u8]) -> Result<String> {
        let hash = sha256_hash(content);
        let path = self.object_path(&hash);

        // Check if already exists (deduplication)
        if path.exists() {
            debug!("Object {} already exists, skipping write", hash);
            return Ok(hash);
        }

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create object directory {:?}", parent))?;
        }

        // Write content
        fs::write(&path, content)
            .with_context(|| format!("Failed to write object {} to {:?}", hash, path))?;

        debug!("Stored object {} ({} bytes)", hash, content.len());
        Ok(hash)
    }

    /// Retrieve content by hash.
    ///
    /// Returns an error if the object doesn't exist.
    pub fn load(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash);
        fs::read(&path).with_context(|| format!("Failed to read object {} from {:?}", hash, path))
    }

    /// Check if content exists.
    pub fn exists(&self, hash: &str) -> bool {
        self.object_path(hash).exists()
    }

    /// Get the path for a given hash.
    ///
    /// Uses git-like structure: `objects/{hash[0..2]}/{hash[2..]}`.
    fn object_path(&self, hash: &str) -> PathBuf {
        // Split hash into directory prefix (first 2 chars) and filename (rest)
        let (prefix, suffix) = if hash.len() >= 2 {
            hash.split_at(2)
        } else {
            // Handle edge case of very short hashes (shouldn't happen with SHA-256)
            (hash, "")
        };
        self.base_path.join(prefix).join(suffix)
    }
}

/// Compute SHA-256 hash of content (hex encoded).
pub fn sha256_hash(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let result = hasher.finalize();
    hex::encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_store() -> (TempDir, ObjectStore) {
        let temp_dir = TempDir::new().unwrap();
        let store = ObjectStore::new(temp_dir.path());
        store.init().unwrap();
        (temp_dir, store)
    }

    #[test]
    fn test_sha256_hash() {
        // Known test vector
        let hash = sha256_hash(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_store_and_load_roundtrip() {
        let (_temp_dir, store) = setup_store();
        let content = b"test content for roundtrip";

        // Store content
        let hash = store.store(content).unwrap();

        // Verify hash is valid
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars

        // Load and verify
        let loaded = store.load(&hash).unwrap();
        assert_eq!(loaded, content);
    }

    #[test]
    fn test_store_deduplication() {
        let (temp_dir, store) = setup_store();
        let content = b"duplicate content test";

        // Store the same content twice
        let hash1 = store.store(content).unwrap();
        let hash2 = store.store(content).unwrap();

        // Should return the same hash
        assert_eq!(hash1, hash2);

        // Count files in the object store to verify no duplicates
        let objects_path = temp_dir.path().join(".dots").join("audit").join("objects");
        let mut file_count = 0;
        for entry in walkdir::WalkDir::new(&objects_path) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                file_count += 1;
            }
        }
        assert_eq!(file_count, 1, "Should only have one file stored");
    }

    #[test]
    fn test_exists() {
        let (_temp_dir, store) = setup_store();
        let content = b"existence test";

        // Should not exist before storing
        let hash = sha256_hash(content);
        assert!(!store.exists(&hash));

        // Store and check again
        store.store(content).unwrap();
        assert!(store.exists(&hash));
    }

    #[test]
    fn test_object_path_structure() {
        let temp_dir = TempDir::new().unwrap();
        let store = ObjectStore::new(temp_dir.path());

        // Test the git-like path structure
        let hash = "abcdef1234567890";
        let path = store.object_path(hash);

        // Should be: base_path/ab/cdef1234567890
        assert!(path.ends_with("ab/cdef1234567890"));
    }

    #[test]
    fn test_load_nonexistent() {
        let (_temp_dir, store) = setup_store();

        // Try to load a hash that doesn't exist
        let result = store.load("0000000000000000000000000000000000000000000000000000000000000000");
        assert!(result.is_err());
    }

    #[test]
    fn test_store_empty_content() {
        let (_temp_dir, store) = setup_store();
        let content = b"";

        // Should be able to store empty content
        let hash = store.store(content).unwrap();
        let loaded = store.load(&hash).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_store_large_content() {
        let (_temp_dir, store) = setup_store();

        // Create a larger piece of content
        let content: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();

        let hash = store.store(&content).unwrap();
        let loaded = store.load(&hash).unwrap();
        assert_eq!(loaded, content);
    }
}
