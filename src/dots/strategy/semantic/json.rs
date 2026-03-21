//! JSON semantic patching.
//!
//! Provides deep merge and RFC 6902 JSON Patch support for JSON files.

use crate::config::{ArrayOp, ArrayOpType, SemanticPatch};
use crate::core::{BombadilError, Result};
use serde_json::{Map, Value};
use std::path::PathBuf;
use tracing::{debug, instrument};

/// Apply semantic patches to a JSON value.
#[instrument(skip(base, patch))]
pub fn apply_patch(base: Value, patch: &SemanticPatch) -> Result<Value> {
    let mut result = base;

    // 1. Apply merge operations (deep merge)
    for (key, value) in &patch.merge {
        result = merge_at_path(&result, key, value.clone())?;
        debug!(key = %key, "Applied merge operation");
    }

    // 2. Apply delete operations
    for key in &patch.delete {
        result = delete_at_path(&result, key)?;
        debug!(key = %key, "Applied delete operation");
    }

    // 3. Apply array operations
    for (key, ops) in &patch.arrays {
        for op in ops {
            result = apply_array_op(&result, key, op)?;
        }
        debug!(key = %key, "Applied array operations");
    }

    // 4. Apply RFC 6902 JSON Patch operations
    if !patch.json_patch.is_empty() {
        let patch_ops: Vec<json_patch::PatchOperation> = patch
            .json_patch
            .iter()
            .map(|v| serde_json::from_value(v.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BombadilError::ConfigInvalid {
                message: format!("Invalid JSON Patch operation: {}", e),
                help: Some("Use RFC 6902 format: {\"op\": \"add\", \"path\": \"/foo\", \"value\": \"bar\"}".to_string()),
            })?;

        let json_patch = json_patch::Patch(patch_ops);
        json_patch::patch(&mut result, &json_patch).map_err(|e| BombadilError::PatchApplyFailed {
            patch_file: PathBuf::new(),
            base_file: PathBuf::new(),
            help: format!("JSON Patch failed: {}", e),
        })?;
        debug!("Applied RFC 6902 JSON Patch operations");
    }

    Ok(result)
}

/// Parse JSON string and apply patches.
pub fn apply_patch_to_string(json_str: &str, patch: &SemanticPatch) -> Result<String> {
    let base: Value = serde_json::from_str(json_str).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Invalid JSON: {}", e),
        help: None,
    })?;

    let result = apply_patch(base, patch)?;

    serde_json::to_string_pretty(&result).map_err(|e| BombadilError::ConfigInvalid {
        message: format!("Failed to serialize JSON: {}", e),
        help: None,
    })
}

/// Deep merge a value at a JSON path.
fn merge_at_path(base: &Value, path: &str, value: Value) -> Result<Value> {
    let mut result = base.clone();

    if path.is_empty() || path == "." {
        // Merge at root
        return Ok(deep_merge(&result, &value));
    }

    // Navigate to path and merge
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut current = &mut result;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part - do the merge
            match current {
                Value::Object(map) => {
                    if let Some(existing) = map.get(*part) {
                        map.insert(part.to_string(), deep_merge(existing, &value));
                    } else {
                        map.insert(part.to_string(), value.clone());
                    }
                }
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        if idx < arr.len() {
                            arr[idx] = deep_merge(&arr[idx], &value);
                        }
                    }
                }
                _ => {}
            }
        } else {
            // Navigate deeper
            current = match current {
                Value::Object(map) => {
                    map.entry(part.to_string())
                        .or_insert(Value::Object(Map::new()))
                }
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        arr.get_mut(idx).ok_or_else(|| BombadilError::ConfigInvalid {
                            message: format!("Array index {} out of bounds at path {}", idx, path),
                            help: None,
                        })?
                    } else {
                        return Err(BombadilError::ConfigInvalid {
                            message: format!("Invalid array index '{}' at path {}", part, path),
                            help: None,
                        });
                    }
                }
                _ => {
                    return Err(BombadilError::ConfigInvalid {
                        message: format!("Cannot navigate through non-container at path {}", path),
                        help: None,
                    });
                }
            };
        }
    }

    Ok(result)
}

/// Deep merge two JSON values.
pub fn deep_merge(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut result = base_map.clone();
            for (key, overlay_value) in overlay_map {
                let merged = if let Some(base_value) = result.get(key) {
                    deep_merge(base_value, overlay_value)
                } else {
                    overlay_value.clone()
                };
                result.insert(key.clone(), merged);
            }
            Value::Object(result)
        }
        // For non-objects, overlay wins
        (_, overlay) => overlay.clone(),
    }
}

/// Delete a value at a JSON path.
fn delete_at_path(base: &Value, path: &str) -> Result<Value> {
    let mut result = base.clone();

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    if parts.is_empty() {
        return Ok(Value::Null);
    }

    // Navigate to parent and remove
    let mut current = &mut result;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part - do the delete
            match current {
                Value::Object(map) => {
                    map.remove(*part);
                }
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        if idx < arr.len() {
                            arr.remove(idx);
                        }
                    }
                }
                _ => {}
            }
        } else {
            // Navigate deeper
            current = match current {
                Value::Object(map) => map.get_mut(*part).ok_or_else(|| BombadilError::ConfigInvalid {
                    message: format!("Path not found: {}", path),
                    help: None,
                })?,
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        arr.get_mut(idx).ok_or_else(|| BombadilError::ConfigInvalid {
                            message: format!("Array index {} out of bounds at path {}", idx, path),
                            help: None,
                        })?
                    } else {
                        return Err(BombadilError::ConfigInvalid {
                            message: format!("Invalid array index '{}' at path {}", part, path),
                            help: None,
                        });
                    }
                }
                _ => {
                    return Err(BombadilError::ConfigInvalid {
                        message: format!("Cannot navigate through non-container at path {}", path),
                        help: None,
                    });
                }
            };
        }
    }

    Ok(result)
}

/// Apply an array operation at a path.
fn apply_array_op(base: &Value, path: &str, op: &ArrayOp) -> Result<Value> {
    let mut result = base.clone();

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut current = &mut result;

    // Navigate to the array
    for part in &parts {
        current = match current {
            Value::Object(map) => map.get_mut(*part).ok_or_else(|| BombadilError::ConfigInvalid {
                message: format!("Path not found: {}", path),
                help: None,
            })?,
            Value::Array(arr) => {
                if let Ok(idx) = part.parse::<usize>() {
                    arr.get_mut(idx).ok_or_else(|| BombadilError::ConfigInvalid {
                        message: format!("Array index {} out of bounds at path {}", idx, path),
                        help: None,
                    })?
                } else {
                    return Err(BombadilError::ConfigInvalid {
                        message: format!("Invalid array index '{}' at path {}", part, path),
                        help: None,
                    });
                }
            }
            _ => {
                return Err(BombadilError::ConfigInvalid {
                    message: format!("Cannot navigate through non-container at path {}", path),
                    help: None,
                });
            }
        };
    }

    // Apply the operation
    match current {
        Value::Array(arr) => {
            match op.operation {
                ArrayOpType::Set => {
                    *arr = match &op.value {
                        Value::Array(new_arr) => new_arr.clone(),
                        _ => vec![op.value.clone()],
                    };
                }
                ArrayOpType::Append => {
                    match &op.value {
                        Value::Array(values) => arr.extend(values.clone()),
                        v => arr.push(v.clone()),
                    }
                }
                ArrayOpType::Prepend => {
                    let mut new_arr = match &op.value {
                        Value::Array(values) => values.clone(),
                        v => vec![v.clone()],
                    };
                    new_arr.extend(arr.drain(..));
                    *arr = new_arr;
                }
                ArrayOpType::Remove => {
                    // Remove matching elements
                    arr.retain(|item| item != &op.value);
                }
            }
        }
        _ => {
            return Err(BombadilError::ConfigInvalid {
                message: format!("Expected array at path {}", path),
                help: None,
            });
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deep_merge_objects() {
        let base = json!({
            "a": 1,
            "b": {
                "c": 2,
                "d": 3
            }
        });

        let overlay = json!({
            "b": {
                "c": 99,
                "e": 4
            },
            "f": 5
        });

        let result = deep_merge(&base, &overlay);

        assert_eq!(result["a"], 1);
        assert_eq!(result["b"]["c"], 99);
        assert_eq!(result["b"]["d"], 3);
        assert_eq!(result["b"]["e"], 4);
        assert_eq!(result["f"], 5);
    }

    #[test]
    fn merge_at_nested_path() {
        let base = json!({
            "settings": {
                "theme": "dark"
            }
        });

        let value = json!({
            "fontSize": 14
        });

        let result = merge_at_path(&base, "/settings", value).unwrap();

        assert_eq!(result["settings"]["theme"], "dark");
        assert_eq!(result["settings"]["fontSize"], 14);
    }

    #[test]
    fn delete_at_path() {
        let base = json!({
            "a": 1,
            "b": {
                "c": 2,
                "d": 3
            }
        });

        let result = super::delete_at_path(&base, "/b/c").unwrap();

        assert_eq!(result["a"], 1);
        assert!(result["b"]["c"].is_null());
        assert_eq!(result["b"]["d"], 3);
    }

    #[test]
    fn array_append() {
        let base = json!({
            "items": [1, 2, 3]
        });

        let op = ArrayOp {
            operation: ArrayOpType::Append,
            value: json!([4, 5]),
        };

        let result = apply_array_op(&base, "/items", &op).unwrap();

        assert_eq!(result["items"], json!([1, 2, 3, 4, 5]));
    }

    #[test]
    fn array_prepend() {
        let base = json!({
            "items": [3, 4, 5]
        });

        let op = ArrayOp {
            operation: ArrayOpType::Prepend,
            value: json!([1, 2]),
        };

        let result = apply_array_op(&base, "/items", &op).unwrap();

        assert_eq!(result["items"], json!([1, 2, 3, 4, 5]));
    }

    #[test]
    fn array_remove() {
        let base = json!({
            "items": [1, 2, 3, 2, 4]
        });

        let op = ArrayOp {
            operation: ArrayOpType::Remove,
            value: json!(2),
        };

        let result = apply_array_op(&base, "/items", &op).unwrap();

        assert_eq!(result["items"], json!([1, 3, 4]));
    }

    #[test]
    fn full_semantic_patch() {
        let base = json!({
            "name": "test",
            "settings": {
                "theme": "light",
                "enabled": true
            },
            "plugins": ["a", "b"]
        });

        let mut merge = std::collections::HashMap::new();
        merge.insert("/settings".to_string(), json!({"theme": "dark", "fontSize": 14}));

        let mut arrays = std::collections::HashMap::new();
        arrays.insert("/plugins".to_string(), vec![ArrayOp {
            operation: ArrayOpType::Append,
            value: json!("c"),
        }]);

        let patch = SemanticPatch {
            format: crate::config::SemanticFormat::Json,
            base: None,
            merge,
            delete: vec![],
            arrays,
            json_patch: vec![],
        };

        let result = apply_patch(base, &patch).unwrap();

        assert_eq!(result["settings"]["theme"], "dark");
        assert_eq!(result["settings"]["enabled"], true);
        assert_eq!(result["settings"]["fontSize"], 14);
        assert_eq!(result["plugins"], json!(["a", "b", "c"]));
    }
}
