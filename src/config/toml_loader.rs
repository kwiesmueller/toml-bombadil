//! TOML configuration loader.
//!
//! This is the primary config format for Bombadil, providing the best
//! editor tooling support through Taplo and JSON Schema.

use crate::config::{loader::ConfigLoader, Config};
use crate::core::{BombadilError, Result};
use std::fs;
use std::path::Path;
use tracing::{debug, instrument};

/// TOML configuration loader.
pub struct TomlLoader;

impl ConfigLoader for TomlLoader {
    #[instrument(skip(self, path))]
    fn load(&self, path: &Path) -> Result<Config> {
        debug!(path = %path.display(), "loading TOML configuration");

        if !path.exists() {
            return Err(BombadilError::ConfigNotFound {
                path: path.to_path_buf(),
            });
        }

        let content = fs::read_to_string(path).map_err(|e| BombadilError::Io {
            context: format!("reading config from {}", path.display()),
            source: e,
        })?;

        let config: Config = toml::from_str(&content).map_err(|e| BombadilError::ConfigParse {
            source: e,
            path: path.to_path_buf(),
        })?;

        debug!(
            dots = config.settings.dots.len(),
            profiles = config.profiles.len(),
            "TOML configuration loaded"
        );

        Ok(config)
    }

    #[instrument(skip(self, config, path))]
    fn save(&self, config: &Config, path: &Path) -> Result<()> {
        debug!(path = %path.display(), "saving TOML configuration");

        let content = toml::to_string_pretty(config).map_err(|e| BombadilError::ConfigInvalid {
            message: format!("Failed to serialize config: {}", e),
            help: None,
        })?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                context: format!("creating parent directory for {}", path.display()),
                source: e,
            })?;
        }

        fs::write(path, content).map_err(|e| BombadilError::Io {
            context: format!("writing config to {}", path.display()),
            source: e,
        })?;

        debug!("TOML configuration saved");
        Ok(())
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["toml"]
    }

    fn generate_schema(&self) -> Option<String> {
        let schema = crate::config::generate_schema();
        serde_json::to_string_pretty(&schema).ok()
    }

    fn format_name(&self) -> &'static str {
        "TOML"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_minimal_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("bombadil.toml");

        fs::write(
            &config_path,
            r#"
            [settings.dots.test]
            source = "test"
            target = "~/.config/test"
        "#,
        )
        .unwrap();

        let loader = TomlLoader;
        let config = loader.load(&config_path).unwrap();

        assert!(config.settings.dots.contains_key("test"));
    }

    #[test]
    fn save_and_reload_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("bombadil.toml");

        let mut config = Config::default();
        config.dotfiles_dir = Some(std::path::PathBuf::from("~/dotfiles"));

        let loader = TomlLoader;
        loader.save(&config, &config_path).unwrap();

        let reloaded = loader.load(&config_path).unwrap();
        assert_eq!(
            reloaded.dotfiles_dir,
            Some(std::path::PathBuf::from("~/dotfiles"))
        );
    }

    #[test]
    fn generate_schema_returns_json() {
        let loader = TomlLoader;
        let schema = loader.generate_schema().unwrap();
        assert!(schema.contains("\"$schema\"") || schema.contains("\"type\""));
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let loader = TomlLoader;
        let result = loader.load(Path::new("/nonexistent/bombadil.toml"));
        assert!(result.is_err());
    }
}
