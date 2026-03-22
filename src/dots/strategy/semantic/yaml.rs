//! YAML semantic patching.
//!
//! Converts YAML to JSON internally for patching, then back to YAML.

use crate::config::SemanticPatch;
use crate::core::{BombadilError, Result};
use serde_json::Value;
use tracing::{debug, instrument};

/// Apply semantic patches to a YAML document.
///
/// Converts YAML to JSON internally for patching, then serializes back to YAML.
#[instrument(skip(yaml_str, patch))]
pub fn apply_patch(yaml_str: &str, patch: &SemanticPatch) -> Result<String> {
    // Parse YAML to serde_json::Value
    let base: Value = serde_yaml::from_str(yaml_str).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Invalid YAML: {}", e),
        help: None,
    })?;

    debug!("Parsed YAML to JSON value");

    // Apply patches using JSON logic
    let result = super::json::apply_patch(base, patch)?;

    debug!("Applied patches to JSON value");

    // Serialize back to YAML
    serde_yaml::to_string(&result).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Failed to serialize YAML: {}", e),
        help: None,
    })
}

/// Parse and validate a YAML string.
pub fn parse(yaml_str: &str) -> Result<Value> {
    serde_yaml::from_str(yaml_str).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Invalid YAML: {}", e),
        help: None,
    })
}

/// Serialize a JSON value to YAML.
pub fn serialize(value: &Value) -> Result<String> {
    serde_yaml::to_string(value).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Failed to serialize YAML: {}", e),
        help: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArrayOp, ArrayOpType, SemanticFormat};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn patch_yaml_merge() {
        let yaml = r#"
name: test
settings:
  theme: light
  enabled: true
"#;

        let mut merge = HashMap::new();
        merge.insert(
            "/settings".to_string(),
            json!({"theme": "dark", "fontSize": 14}),
        );

        let patch = SemanticPatch {
            format: SemanticFormat::Yaml,
            base: None,
            merge,
            delete: vec![],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(yaml, &patch).unwrap();

        assert!(result.contains("theme: dark"));
        assert!(result.contains("enabled: true"));
        assert!(result.contains("fontSize: 14"));
    }

    #[test]
    fn patch_yaml_delete() {
        let yaml = r#"
name: test
remove_me: value
keep: true
"#;

        let patch = SemanticPatch {
            format: SemanticFormat::Yaml,
            base: None,
            merge: HashMap::new(),
            delete: vec!["/remove_me".to_string()],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(yaml, &patch).unwrap();

        assert!(!result.contains("remove_me"));
        assert!(result.contains("name: test"));
        assert!(result.contains("keep: true"));
    }

    #[test]
    fn patch_yaml_array() {
        let yaml = r#"
items:
  - a
  - b
"#;

        let mut arrays = HashMap::new();
        arrays.insert(
            "/items".to_string(),
            vec![ArrayOp {
                operation: ArrayOpType::Append,
                value: json!("c"),
            }],
        );

        let patch = SemanticPatch {
            format: SemanticFormat::Yaml,
            base: None,
            merge: HashMap::new(),
            delete: vec![],
            arrays,
            json_patch: vec![],
        };

        let result = apply_patch(yaml, &patch).unwrap();

        // Check that all items are present
        assert!(result.contains("- a"));
        assert!(result.contains("- b"));
        assert!(result.contains("- c"));
    }

    #[test]
    fn roundtrip_preserves_structure() {
        let yaml = r#"
name: test
nested:
  key: value
  list:
    - item1
    - item2
"#;

        let patch = SemanticPatch {
            format: SemanticFormat::Yaml,
            base: None,
            merge: HashMap::new(),
            delete: vec![],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(yaml, &patch).unwrap();

        // Parse and reserialize to verify structure
        let value: Value = serde_yaml::from_str(&result).unwrap();
        assert_eq!(value["name"], "test");
        assert_eq!(value["nested"]["key"], "value");
        assert_eq!(value["nested"]["list"][0], "item1");
        assert_eq!(value["nested"]["list"][1], "item2");
    }
}
