//! INI semantic patching.
//!
//! Provides basic section/key-level patching for INI files.

use crate::config::SemanticPatch;
use crate::core::{BombadilError, Result};
use serde_json::{Map, Value};
use tracing::{debug, instrument};

/// Apply semantic patches to an INI file.
///
/// INI files are converted to a nested structure where:
/// - Top-level keys represent sections (or "" for sectionless keys)
/// - Each section is an object with key-value pairs
#[instrument(skip(ini_str, patch))]
pub fn apply_patch(ini_str: &str, patch: &SemanticPatch) -> Result<String> {
    // Parse INI to nested JSON structure
    let base = parse_ini(ini_str)?;

    debug!("Parsed INI to JSON value");

    // Apply patches using JSON logic
    let result = super::json::apply_patch(base, patch)?;

    debug!("Applied patches to JSON value");

    // Serialize back to INI format
    serialize_ini(&result)
}

/// Parse INI string to JSON value.
///
/// Structure:
/// ```json
/// {
///   "": { "global_key": "value" },  // Keys before any section
///   "section1": { "key": "value" },
///   "section2": { "key": "value" }
/// }
/// ```
fn parse_ini(ini_str: &str) -> Result<Value> {
    let mut result: Map<String, Value> = Map::new();
    let mut current_section = String::new();

    // Initialize global section
    result.insert(String::new(), Value::Object(Map::new()));

    for line in ini_str.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            if !result.contains_key(&current_section) {
                result.insert(current_section.clone(), Value::Object(Map::new()));
            }
            continue;
        }

        // Key = value
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            // Remove quotes if present
            let value = if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };

            if let Some(section) = result.get_mut(&current_section) {
                if let Some(section_map) = section.as_object_mut() {
                    section_map.insert(key.to_string(), Value::String(value.to_string()));
                }
            }
        }
    }

    // Remove empty global section if no global keys
    if let Some(global) = result.get("") {
        if global.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            result.remove("");
        }
    }

    Ok(Value::Object(result))
}

/// Serialize JSON value back to INI format.
fn serialize_ini(value: &Value) -> Result<String> {
    let mut result = String::new();

    let obj = value.as_object().ok_or_else(|| BombadilError::ConfigInvalid {
        message: "INI root must be an object".to_string(),
        help: None,
    })?;

    // Write global keys first (empty section name)
    if let Some(global) = obj.get("") {
        if let Some(global_map) = global.as_object() {
            for (key, value) in global_map {
                write_ini_value(&mut result, key, value);
            }
            if !global_map.is_empty() {
                result.push('\n');
            }
        }
    }

    // Write sections
    let mut sections: Vec<_> = obj.keys().filter(|k| !k.is_empty()).collect();
    sections.sort();

    for section in sections {
        if let Some(section_obj) = obj.get(section).and_then(|v| v.as_object()) {
            result.push('[');
            result.push_str(section);
            result.push_str("]\n");

            for (key, value) in section_obj {
                write_ini_value(&mut result, key, value);
            }

            result.push('\n');
        }
    }

    // Remove trailing newline
    while result.ends_with("\n\n") {
        result.pop();
    }

    Ok(result)
}

/// Write a single INI key-value pair.
fn write_ini_value(result: &mut String, key: &str, value: &Value) {
    result.push_str(key);
    result.push_str(" = ");

    match value {
        Value::String(s) => {
            // Quote strings with special characters
            if s.contains(' ') || s.contains('=') || s.contains('#') || s.contains(';') {
                result.push('"');
                result.push_str(s);
                result.push('"');
            } else {
                result.push_str(s);
            }
        }
        Value::Number(n) => result.push_str(&n.to_string()),
        Value::Bool(b) => result.push_str(if *b { "true" } else { "false" }),
        Value::Null => result.push_str("null"),
        Value::Array(_) | Value::Object(_) => {
            // INI doesn't support nested structures - serialize as JSON
            if let Ok(json) = serde_json::to_string(value) {
                result.push_str(&json);
            }
        }
    }

    result.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SemanticFormat;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn parse_simple_ini() {
        let ini = r#"
[section1]
key1 = value1
key2 = value2

[section2]
key3 = value3
"#;

        let result = parse_ini(ini).unwrap();

        assert_eq!(result["section1"]["key1"], "value1");
        assert_eq!(result["section1"]["key2"], "value2");
        assert_eq!(result["section2"]["key3"], "value3");
    }

    #[test]
    fn parse_ini_with_global_keys() {
        let ini = r#"
global_key = global_value

[section1]
key1 = value1
"#;

        let result = parse_ini(ini).unwrap();

        assert_eq!(result[""]["global_key"], "global_value");
        assert_eq!(result["section1"]["key1"], "value1");
    }

    #[test]
    fn parse_ini_with_quotes() {
        let ini = r#"
[section]
key1 = "quoted value"
key2 = 'single quoted'
key3 = unquoted
"#;

        let result = parse_ini(ini).unwrap();

        assert_eq!(result["section"]["key1"], "quoted value");
        assert_eq!(result["section"]["key2"], "single quoted");
        assert_eq!(result["section"]["key3"], "unquoted");
    }

    #[test]
    fn parse_ini_with_comments() {
        let ini = r#"
# This is a comment
; Another comment

[section]
key = value
# Inline comment style not supported - this line is skipped
"#;

        let result = parse_ini(ini).unwrap();

        assert_eq!(result["section"]["key"], "value");
    }

    #[test]
    fn serialize_ini() {
        let value = json!({
            "section1": {
                "key1": "value1",
                "key2": "value2"
            },
            "section2": {
                "key3": "value3"
            }
        });

        let result = super::serialize_ini(&value).unwrap();

        assert!(result.contains("[section1]"));
        assert!(result.contains("key1 = value1"));
        assert!(result.contains("[section2]"));
        assert!(result.contains("key3 = value3"));
    }

    #[test]
    fn patch_ini_merge() {
        let ini = r#"
[section]
key1 = old_value
key2 = keep_this
"#;

        let mut merge = HashMap::new();
        merge.insert("/section".to_string(), json!({"key1": "new_value", "key3": "added"}));

        let patch = SemanticPatch {
            format: SemanticFormat::Ini,
            base: None,
            merge,
            delete: vec![],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(ini, &patch).unwrap();

        assert!(result.contains("key1 = new_value"));
        assert!(result.contains("key2 = keep_this"));
        assert!(result.contains("key3 = added"));
    }

    #[test]
    fn patch_ini_delete() {
        let ini = r#"
[section]
keep = yes
remove = no
"#;

        let patch = SemanticPatch {
            format: SemanticFormat::Ini,
            base: None,
            merge: HashMap::new(),
            delete: vec!["/section/remove".to_string()],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(ini, &patch).unwrap();

        assert!(result.contains("keep = yes"));
        assert!(!result.contains("remove"));
    }

    #[test]
    fn roundtrip_preserves_structure() {
        let ini = r#"
[desktop]
Name = Application
Exec = /usr/bin/app
Icon = app

[settings]
Theme = dark
"#;

        let patch = SemanticPatch {
            format: SemanticFormat::Ini,
            base: None,
            merge: HashMap::new(),
            delete: vec![],
            arrays: HashMap::new(),
            json_patch: vec![],
        };

        let result = apply_patch(ini, &patch).unwrap();

        // Parse result and verify structure
        let parsed = parse_ini(&result).unwrap();
        assert_eq!(parsed["desktop"]["Name"], "Application");
        assert_eq!(parsed["desktop"]["Exec"], "/usr/bin/app");
        assert_eq!(parsed["settings"]["Theme"], "dark");
    }
}
