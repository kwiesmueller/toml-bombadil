//! Config loader trait for pluggable config format support.
//!
//! This module defines the `ConfigLoader` trait that enables support for
//! multiple configuration formats (TOML, KDL, YAML, etc.).

use crate::config::Config;
use crate::core::Result;
use std::path::Path;

/// Trait for loading and saving configuration files.
///
/// Implementations of this trait provide support for different config formats.
/// The internal representation is always the same (`Config`), but the on-disk
/// format can vary.
pub trait ConfigLoader: Send + Sync {
    /// Load configuration from a file path.
    fn load(&self, path: &Path) -> Result<Config>;

    /// Save configuration to a file path.
    fn save(&self, config: &Config, path: &Path) -> Result<()>;

    /// Get the file extensions this loader handles.
    ///
    /// Used to auto-detect the appropriate loader based on file extension.
    fn file_extensions(&self) -> &[&'static str];

    /// Generate a schema for this format (if supported).
    ///
    /// Returns `Some(schema_string)` for formats that support schema generation
    /// (e.g., JSON Schema for TOML/JSON/YAML), or `None` for formats without
    /// schema support.
    fn generate_schema(&self) -> Option<String>;

    /// Get the name of this config format.
    fn format_name(&self) -> &'static str;
}

/// Registry of available config loaders.
pub struct LoaderRegistry {
    loaders: Vec<Box<dyn ConfigLoader>>,
}

impl Default for LoaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LoaderRegistry {
    /// Create a new registry with all built-in loaders.
    pub fn new() -> Self {
        let mut registry = Self {
            loaders: Vec::new(),
        };

        // Register built-in loaders
        registry.register(Box::new(super::toml_loader::TomlLoader));

        registry
    }

    /// Register a custom loader.
    pub fn register(&mut self, loader: Box<dyn ConfigLoader>) {
        self.loaders.push(loader);
    }

    /// Find a loader for the given file extension.
    pub fn loader_for_extension(&self, ext: &str) -> Option<&dyn ConfigLoader> {
        self.loaders
            .iter()
            .find(|loader| loader.file_extensions().contains(&ext))
            .map(|b| b.as_ref())
    }

    /// Find a loader for the given path (based on extension).
    pub fn loader_for_path(&self, path: &Path) -> Option<&dyn ConfigLoader> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| self.loader_for_extension(ext))
    }

    /// Get all registered loaders.
    pub fn loaders(&self) -> &[Box<dyn ConfigLoader>] {
        &self.loaders
    }

    /// Load config from a path, auto-detecting the format.
    pub fn load(&self, path: &Path) -> Result<Config> {
        match self.loader_for_path(path) {
            Some(loader) => loader.load(path),
            None => {
                // Default to TOML for unknown extensions
                self.loaders
                    .first()
                    .expect("at least one loader registered")
                    .load(path)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_toml_loader() {
        let registry = LoaderRegistry::new();
        assert!(registry.loader_for_extension("toml").is_some());
    }

    #[test]
    fn toml_loader_handles_toml_extension() {
        let registry = LoaderRegistry::new();
        let loader = registry.loader_for_extension("toml").unwrap();
        assert_eq!(loader.format_name(), "TOML");
    }
}
