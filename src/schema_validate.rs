//! Minimal JSON-Schema validation for `--output-schema`: the supported
//! subset is `type`, `properties`, `required`, `items`, and `enum` (nested
//! arbitrarily). No dependencies — pipeline output needs shape checking, not
//! the full spec — and every error names the exact path and what to fix.

use serde_json::Value;

/// Validate `value` against a schema subset. Errors are written for the
/// model to act on: they name the property path and the mismatch.
pub fn validate(value: &Value, schema: &Value) -> Result<(), String> {
    validate_at(value, schema, "$")
}

fn validate_at(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(obj) = schema.as_object() else {
        return Ok(()); // not a schema object → nothing to check
    };
    if let Some(expected) = obj.get("type").and_then(|t| t.as_str()) {
        check_type(value, expected, path)?;
    }
    if let Some(allowed) = obj.get("enum").and_then(|e| e.as_array()) {
        if !allowed.contains(value) {
            return Err(format!(
                "{path}: value {value} is not one of the allowed enum values {allowed:?}"
            ));
        }
    }
    if let (Some(props), Some(map)) = (obj.get("properties"), value.as_object()) {
        for (name, prop_schema) in props.as_object().into_iter().flatten() {
            if let Some(v) = map.get(name) {
                validate_at(v, prop_schema, &format!("{path}.{name}"))?;
            }
        }
    }
    if let Some(required) = obj.get("required").and_then(|r| r.as_array()) {
        if let Some(map) = value.as_object() {
            for name in required.iter().filter_map(|n| n.as_str()) {
                if !map.contains_key(name) {
                    return Err(format!("{path}: missing required property '{name}'"));
                }
            }
        }
    }
    if let Some(item_schema) = obj.get("items") {
        if let Some(arr) = value.as_array() {
            for (i, item) in arr.iter().enumerate() {
                validate_at(item, item_schema, &format!("{path}[{i}]"))?;
            }
        }
    }
    Ok(())
}

fn check_type(value: &Value, expected: &str, path: &str) -> Result<(), String> {
    let ok = match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        other => {
            return Err(format!(
                "{path}: schema uses unsupported type '{other}' (supported: object, array, \
                 string, boolean, integer, number, null)"
            ))
        }
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{path}: expected {expected}, got {}",
            type_name(value)
        ))
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Extract JSON from a model's final message: accepts bare JSON or a
/// fenced ```/```json block (models like wrapping it). Returns the parsed
/// value or an error saying what to do.
pub fn parse_output(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    let candidate = if trimmed.starts_with("```") {
        let body = trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches('`')
            .trim();
        body
    } else {
        trimmed
    };
    serde_json::from_str(candidate).map_err(|e| format!(
        "output is not valid JSON ({e}). Output ONLY the JSON value — no prose, no code fences."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn person_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "role": {"enum": ["admin", "user"]}
            },
            "required": ["name", "role"]
        })
    }

    #[test]
    fn accepts_valid_and_rejects_each_way() {
        let schema = person_schema();
        assert!(validate(&json!({"name": "x", "role": "admin", "age": 3, "tags": ["a"]}), &schema).is_ok());
        assert_eq!(
            validate(&json!({"role": "admin"}), &schema).unwrap_err(),
            "$: missing required property 'name'"
        );
        assert!(validate(&json!({"name": "x", "role": "admin", "age": "old"}), &schema)
            .unwrap_err()
            .contains("$.age: expected integer, got string"));
        assert!(validate(&json!({"name": "x", "role": "wizard"}), &schema)
            .unwrap_err()
            .contains("not one of the allowed enum values"));
        assert!(validate(&json!({"name": "x", "role": "admin", "tags": [1]}), &schema)
            .unwrap_err()
            .contains("$.tags[0]: expected string, got integer"));
    }

    #[test]
    fn parse_output_strips_fences() {
        assert!(parse_output("{\"a\": 1}").is_ok());
        assert!(parse_output("```json\n{\"a\": 1}\n```").is_ok());
        assert!(parse_output("```\n{\"a\": 1}\n```").is_ok());
        assert!(parse_output("here is the json: {\"a\": 1}").is_err());
    }

    #[test]
    fn unsupported_type_is_loud() {
        let err = validate(&json!(1), &json!({"type": "anyOf"})).unwrap_err();
        assert!(err.contains("unsupported type 'anyOf'"), "{err}");
    }
}
