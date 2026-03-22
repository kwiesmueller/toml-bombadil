//! TOML semantic patching.
//!
//! Uses toml_edit to preserve formatting where possible.

use crate::config::SemanticPatch;
use crate::core::{BombadilError, Result};
use serde_json::Value;
use tracing::{debug, instrument};

/// Apply semantic patches to a TOML document.
///
/// Converts TOML to JSON internally for patching, then serializes back to TOML.
/// Uses standard TOML serialization (toml_edit could be used for format preservation
/// but adds complexity).
#[instrument(skip(toml_str, patch))]
pub fn apply_patch(toml_str: &str, patch: &SemanticPatch) -> Result<String> {
    // Parse TOML to serde_json::Value
    let base: Value = toml::from_str(toml_str).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Invalid TOML: {}", e),
        help: None,
    })?;

    debug!("Parsed TOML to JSON value");

    // Apply patches using JSON logic
    let result = super::json::apply_patch(base, patch)?;

    debug!("Applied patches to JSON value");

    // Serialize back to TOML
    toml::to_string_pretty(&result).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Failed to serialize TOML: {}", e),
        help: None,
    })
}

/// Parse and validate a TOML string.
pub fn parse(toml_str: &str) -> Result<Value> {
    toml::from_str(toml_str).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Invalid TOML: {}", e),
        help: None,
    })
}

/// Serialize a JSON value to TOML.
pub fn serialize(value: &Value) -> Result<String> {
    toml::to_string_pretty(value).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Failed to serialize TOML: {}", e),
        help: None,
    })
}

/// Apply patch using toml_edit to preserve formatting.
///
/// This is more complex but preserves comments and whitespace.
#[instrument(skip(toml_str, patch))]
pub fn apply_patch_preserving(toml_str: &str, patch: &SemanticPatch) -> Result<String> {
    use toml_edit::DocumentMut;

    let mut doc: DocumentMut =
        toml_str
            .parse()
            .map_err(|e: toml_edit::TomlError| BombadilError::ConfigInvalid {
                message: format!("Invalid TOML: {}", e),
                help: None,
            })?;

    // Apply merge operations
    for (path, value) in &patch.merge {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if let Some(last) = parts.last() {
            if let Some(parent_item) = navigate_to_parent(&mut doc, &parts[..parts.len() - 1]) {
                if let Some(table) = parent_item.as_table_mut() {
                    // Convert serde_json::Value to toml_edit::Item
                    if let Some(toml_value) = json_to_toml_edit(value) {
                        table.insert(last, toml_value);
                        debug!(path = %path, "Applied merge with formatting preservation");
                    }
                }
            }
        }
    }

    // Apply delete operations
    for path in &patch.delete {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if let Some(last) = parts.last() {
            if let Some(parent_item) = navigate_to_parent(&mut doc, &parts[..parts.len() - 1]) {
                if let Some(table) = parent_item.as_table_mut() {
                    table.remove(last);
                    debug!(path = %path, "Applied delete with formatting preservation");
                }
            }
        }
    }

    Ok(doc.to_string())
}

/// Navigate to parent item in document.
fn navigate_to_parent<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    parts: &[&str],
) -> Option<&'a mut toml_edit::Item> {
    let mut current: &mut toml_edit::Item = doc.as_item_mut();

    for part in parts {
        current = match current {
            toml_edit::Item::Table(table) => table.get_mut(part)?,
            _ => return None,
        };
    }

    Some(current)
}

/// Convert serde_json::Value to toml_edit::Item.
fn json_to_toml_edit(value: &Value) -> Option<toml_edit::Item> {
    use toml_edit::{Array, InlineTable, Item, Value as TomlValue};

    match value {
        Value::Null => None,
        Value::Bool(b) => Some(Item::Value(TomlValue::Boolean(toml_edit::Formatted::new(
            *b,
        )))),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Item::Value(TomlValue::Integer(toml_edit::Formatted::new(
                    i,
                ))))
            } else { n.as_f64().map(|f| Item::Value(TomlValue::Float(toml_edit::Formatted::new(f)))) }
        }
        Value::String(s) => Some(Item::Value(TomlValue::String(toml_edit::Formatted::new(
            s.clone(),
        )))),
        Value::Array(arr) => {
            let mut toml_arr = Array::new();
            for item in arr {
                if let Some(Item::Value(v)) = json_to_toml_edit(item) {
                    toml_arr.push(v);
                }
            }
            Some(Item::Value(TomlValue::Array(toml_arr)))
        }
        Value::Object(obj) => {
            let mut table = InlineTable::new();
            for (k, v) in obj {
                if let Some(Item::Value(v)) = json_to_toml_edit(v) {
                    table.insert(k, v);
                }
            }
            Some(Item::Value(TomlValue::InlineTable(table)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArrayOp, ArrayOpType, SemanticFormat};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn patch_toml_merge() {
        let toml = r#"
name = "test"

[settings]
theme = "light"
enabled = true
"#;

        let mut merge = HashMap::new();
        merge.insert(
            "/settings".to_string(),
            json!({"theme": "dark", "fontSize": 14}),
        );

        let patch = SemanticPatch {
            format: SemanticFormat::Toml,
            base: None,
            merge,
            delete: vec![],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(toml, &patch).unwrap();

        assert!(result.contains("theme = \"dark\"") || result.contains("theme = 'dark'"));
        assert!(result.contains("enabled = true"));
        assert!(result.contains("fontSize = 14"));
    }

    #[test]
    fn patch_toml_delete() {
        let toml = r#"
name = "test"
remove_me = "value"
keep = true
"#;

        let patch = SemanticPatch {
            format: SemanticFormat::Toml,
            base: None,
            merge: HashMap::new(),
            delete: vec!["/remove_me".to_string()],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(toml, &patch).unwrap();

        assert!(!result.contains("remove_me"));
        assert!(result.contains("name = \"test\"") || result.contains("name = 'test'"));
        assert!(result.contains("keep = true"));
    }

    #[test]
    fn patch_toml_array() {
        let toml = r#"
items = ["a", "b"]
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
            format: SemanticFormat::Toml,
            base: None,
            merge: HashMap::new(),
            delete: vec![],
            arrays,
            json_patch: vec![],
        };

        let result = apply_patch(toml, &patch).unwrap();

        // TOML arrays might be formatted differently, just check all elements exist
        assert!(result.contains("\"a\"") || result.contains("'a'"));
        assert!(result.contains("\"b\"") || result.contains("'b'"));
        assert!(result.contains("\"c\"") || result.contains("'c'"));
    }

    #[test]
    fn preserving_patch_merge() {
        let toml = r#"
# This is a comment
name = "test"

[settings]
# Theme setting
theme = "light"
"#;

        let mut merge = HashMap::new();
        merge.insert("/settings/fontSize".to_string(), json!(14));

        let patch = SemanticPatch {
            format: SemanticFormat::Toml,
            base: None,
            merge,
            delete: vec![],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch_preserving(toml, &patch).unwrap();

        // Comments should be preserved
        assert!(result.contains("# This is a comment"));
        // Original values should be present
        assert!(result.contains("name = \"test\""));
    }
}
