use jsonschema::JSONSchema;
use serde_json::Value;

pub const MAX_SCHEMA_BYTES: usize = 16 * 1024;

pub struct ResultSpec {
    pub schema: Value,
    compiled: JSONSchema,
}

impl std::fmt::Debug for ResultSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultSpec")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl ResultSpec {
    pub fn new(schema: Value) -> Result<Self, String> {
        Self::enforce_size_cap(&schema)?;
        let compiled = JSONSchema::compile(&schema)
            .map_err(|e| format!("invalid JSON Schema: {e}"))?;
        Ok(Self { schema, compiled })
    }

    pub fn enforce_size_cap(schema: &Value) -> Result<(), String> {
        let size = serde_json::to_string(schema).map(|s| s.len()).unwrap_or(0);
        if size > MAX_SCHEMA_BYTES {
            Err(format!(
                "result_schema exceeds maximum size of {MAX_SCHEMA_BYTES} bytes (got {size})"
            ))
        } else {
            Ok(())
        }
    }

    pub fn validate(&self, result: &str) -> Result<(), String> {
        let target = self.parse(result)?;
        self.validate_value(&target)
    }

    pub fn validate_value(&self, value: &Value) -> Result<(), String> {
        self.compiled.validate(value).map_err(|errors| {
            errors
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    /// Type=string schemas accept the raw input; everything else expects JSON encoding.
    pub fn parse(&self, result: &str) -> Result<Value, String> {
        if matches!(
            self.schema.get("type").and_then(|v| v.as_str()),
            Some("string")
        ) {
            Ok(Value::String(result.to_string()))
        } else {
            serde_json::from_str(result).map_err(|e| {
                format!("result must be a JSON-encoded value matching the schema: {e}")
            })
        }
    }
}

pub fn validate_schema_doc(schema: &Value) -> Result<(), String> {
    ResultSpec::new(schema.clone()).map(|_| ())
}

fn is_scalar_type(t: &str) -> bool {
    matches!(t, "string" | "number" | "integer" | "boolean" | "null")
}

/// A schema is "simple-renderable" without inference when its top-level shape is a scalar,
/// a scalar-or-null union, an array of scalars, a oneOf/anyOf whose branches are simple,
/// or an object whose direct properties are each themselves simple branches (one level deep).
pub fn is_simple_schema(schema: &Value) -> bool {
    if is_simple_branch(schema) {
        return true;
    }
    if schema.get("type").and_then(|v| v.as_str()) == Some("object") {
        let props = schema.get("properties").and_then(|v| v.as_object());
        return match props {
            Some(p) => p.values().all(is_simple_branch),
            None => false,
        };
    }
    false
}

fn is_simple_branch(schema: &Value) -> bool {
    if let Some(t) = schema.get("type") {
        match t {
            Value::String(s) => {
                if is_scalar_type(s) {
                    return true;
                }
                if s == "array" {
                    return schema.get("items").is_some_and(is_simple_branch);
                }
                return false;
            }
            Value::Array(types) => {
                let mut has_array = false;
                for v in types {
                    match v.as_str() {
                        Some(n) if is_scalar_type(n) => {}
                        Some("array") => has_array = true,
                        _ => return false,
                    }
                }
                if has_array {
                    return schema.get("items").is_some_and(is_simple_branch);
                }
                return true;
            }
            _ => {}
        }
    }
    if let Some(Value::Array(branches)) = schema.get("oneOf").or_else(|| schema.get("anyOf")) {
        return branches.iter().all(is_simple_branch);
    }
    false
}

/// Returns `None` to signal "no delivery" (null, empty object, empty array).
pub fn render_result(schema: &Value, value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }
    match value {
        Value::Array(arr) => {
            if arr.is_empty() {
                return None;
            }
            Some(
                arr.iter()
                    .map(|v| format!("- {}", render_value(v)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        Value::Object(obj) => {
            if obj.is_empty() {
                return None;
            }
            let props = find_object_properties(schema);
            let mut lines: Vec<(String, String)> = Vec::new();
            if let Some(props) = props {
                for (key, prop_schema) in props {
                    match obj.get(key) {
                        Some(v) if !v.is_null() => {
                            let label = prop_schema
                                .get("description")
                                .and_then(|d| d.as_str())
                                .unwrap_or(key.as_str())
                                .to_string();
                            lines.push((label, render_value(v)));
                        }
                        _ => {}
                    }
                }
            } else {
                for (key, v) in obj {
                    if !v.is_null() {
                        lines.push((key.clone(), render_value(v)));
                    }
                }
            }
            if lines.is_empty() {
                return None;
            }
            if lines.len() == 1 {
                return Some(lines.remove(0).1);
            }
            Some(
                lines
                    .iter()
                    .map(|(label, val)| format!("{label}: {val}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        _ => Some(render_value(value)),
    }
}

fn find_object_properties(schema: &Value) -> Option<&serde_json::Map<String, Value>> {
    if let Some(p) = schema.get("properties").and_then(|v| v.as_object()) {
        return Some(p);
    }
    if let Some(Value::Array(branches)) = schema.get("oneOf").or_else(|| schema.get("anyOf")) {
        for branch in branches {
            if let Some(p) = branch.get("properties").and_then(|v| v.as_object()) {
                return Some(p);
            }
        }
    }
    None
}

fn render_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Array(arr) => arr
            .iter()
            .map(render_value)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_accepts_string_with_pattern() {
        ResultSpec::new(json!({"type": "string", "pattern": "^[0-9]{6}$"})).unwrap();
    }

    #[test]
    fn new_accepts_string_with_enum() {
        ResultSpec::new(json!({"type": "string", "enum": ["yes", "no"]})).unwrap();
    }

    #[test]
    fn new_accepts_object_with_required() {
        ResultSpec::new(json!({
            "type": "object",
            "properties": {
                "code": {"type": "string"},
            },
            "required": ["code"]
        }))
        .unwrap();
    }

    #[test]
    fn new_rejects_invalid_pattern() {
        let err = ResultSpec::new(json!({"type": "string", "pattern": "[unterminated"}))
            .unwrap_err();
        assert!(err.contains("invalid JSON Schema"));
    }

    #[test]
    fn validate_string_accepts_matching_pattern() {
        let spec = ResultSpec::new(json!({"type": "string", "pattern": "^[0-9]{6}$"})).unwrap();
        spec.validate("123456").unwrap();
    }

    #[test]
    fn validate_string_rejects_non_matching_pattern() {
        let spec = ResultSpec::new(json!({"type": "string", "pattern": "^[0-9]{6}$"})).unwrap();
        assert!(spec.validate("12345").is_err());
        assert!(spec.validate("abc123").is_err());
    }

    #[test]
    fn validate_string_enum_only_accepts_listed_values() {
        let spec = ResultSpec::new(json!({"type": "string", "enum": ["yes", "no"]})).unwrap();
        spec.validate("yes").unwrap();
        spec.validate("no").unwrap();
        assert!(spec.validate("maybe").is_err());
    }

    #[test]
    fn validate_object_parses_json_and_checks_fields() {
        let spec = ResultSpec::new(json!({
            "type": "object",
            "properties": {
                "is_important": {"type": "string", "enum": ["yes", "no"]},
                "category": {"type": "string"}
            },
            "required": ["is_important", "category"],
            "additionalProperties": false
        }))
        .unwrap();

        spec.validate(r#"{"is_important":"yes","category":"dismissal"}"#)
            .unwrap();
    }

    #[test]
    fn validate_object_rejects_missing_required_field() {
        let spec = ResultSpec::new(json!({
            "type": "object",
            "properties": {
                "is_important": {"type": "string"},
                "category": {"type": "string"}
            },
            "required": ["is_important", "category"]
        }))
        .unwrap();
        let err = spec
            .validate(r#"{"is_important":"yes"}"#)
            .unwrap_err();
        assert!(err.contains("category"), "error should name missing field: {err}");
    }

    #[test]
    fn validate_object_rejects_malformed_json() {
        let spec = ResultSpec::new(json!({"type": "object"})).unwrap();
        let err = spec.validate("not-json").unwrap_err();
        assert!(err.contains("JSON-encoded"));
    }

    #[test]
    fn validate_nested_object_checks_subfields() {
        let spec = ResultSpec::new(json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {"inner": {"type": "string"}},
                    "required": ["inner"]
                }
            },
            "required": ["outer"]
        }))
        .unwrap();

        spec.validate(r#"{"outer":{"inner":"hi"}}"#).unwrap();
        assert!(spec.validate(r#"{"outer":{}}"#).is_err());
        assert!(spec.validate(r#"{"outer":{"inner":42}}"#).is_err());
    }

    #[test]
    fn enforce_size_cap_rejects_oversized() {
        let huge_string: String = "x".repeat(MAX_SCHEMA_BYTES + 1);
        let schema = json!({"type": "string", "description": huge_string});
        assert!(ResultSpec::enforce_size_cap(&schema).is_err());
    }

    #[test]
    fn enforce_size_cap_accepts_small_doc() {
        let schema = json!({"type": "string"});
        ResultSpec::enforce_size_cap(&schema).unwrap();
    }

    #[test]
    fn validate_schema_doc_round_trip() {
        validate_schema_doc(&json!({"type": "string"})).unwrap();
        assert!(validate_schema_doc(&json!({"type": "string", "pattern": "[bad"})).is_err());

        let huge: String = "x".repeat(MAX_SCHEMA_BYTES + 1);
        assert!(validate_schema_doc(&json!({"type": "string", "description": huge})).is_err());
    }

    #[test]
    fn is_simple_scalar_types() {
        for t in ["string", "number", "integer", "boolean", "null"] {
            assert!(is_simple_schema(&json!({"type": t})), "type={t}");
        }
    }

    #[test]
    fn is_simple_nullable_scalar() {
        assert!(is_simple_schema(&json!({"type": ["string", "null"]})));
        assert!(is_simple_schema(&json!({"type": ["number", "null"]})));
    }

    #[test]
    fn is_simple_array_of_scalars() {
        assert!(is_simple_schema(&json!({"type": "array", "items": {"type": "string"}})));
        assert!(is_simple_schema(
            &json!({"type": ["array", "null"], "items": {"type": "string"}})
        ));
    }

    #[test]
    fn is_simple_oneof_scalars_and_null() {
        assert!(is_simple_schema(&json!({
            "oneOf": [{"type": "null"}, {"type": "string"}, {"type": "number"}]
        })));
    }

    #[test]
    fn is_simple_object_with_scalar_props() {
        assert!(is_simple_schema(&json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string"},
                "price": {"type": "number"},
                "change_pct": {"type": "number"}
            },
            "required": ["symbol", "price"]
        })));
    }

    #[test]
    fn is_complex_nested_object() {
        assert!(!is_simple_schema(&json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}}
                }
            }
        })));
    }

    #[test]
    fn is_complex_array_of_objects() {
        assert!(!is_simple_schema(&json!({
            "type": "array",
            "items": {"type": "object", "properties": {"x": {"type": "string"}}}
        })));
    }

    #[test]
    fn is_complex_object_without_properties() {
        assert!(!is_simple_schema(&json!({"type": "object"})));
    }

    #[test]
    fn render_scalar_string() {
        let schema = json!({"type": "string"});
        assert_eq!(
            render_result(&schema, &json!("hello")),
            Some("hello".to_string())
        );
    }

    #[test]
    fn render_scalar_number() {
        let schema = json!({"type": "number"});
        assert_eq!(render_result(&schema, &json!(42)), Some("42".to_string()));
        assert_eq!(render_result(&schema, &json!(3.14)), Some("3.14".to_string()));
    }

    #[test]
    fn render_nullable_string_with_null_returns_none() {
        let schema = json!({"type": ["string", "null"]});
        assert_eq!(render_result(&schema, &Value::Null), None);
    }

    #[test]
    fn render_nullable_string_with_value() {
        let schema = json!({"type": ["string", "null"]});
        assert_eq!(
            render_result(&schema, &json!("emergency")),
            Some("emergency".to_string())
        );
    }

    #[test]
    fn render_array_of_scalars_bullet_list() {
        let schema = json!({"type": "array", "items": {"type": "string"}});
        assert_eq!(
            render_result(&schema, &json!(["a", "b", "c"])),
            Some("- a\n- b\n- c".to_string())
        );
    }

    #[test]
    fn render_empty_array_returns_none() {
        let schema = json!({"type": "array", "items": {"type": "string"}});
        assert_eq!(render_result(&schema, &json!([])), None);
    }

    #[test]
    fn render_object_multi_prop_uses_descriptions_as_labels() {
        let schema = json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string", "description": "ticker"},
                "price": {"type": "number", "description": "current price (USD)"}
            }
        });
        let value = json!({"symbol": "AAPL", "price": 234});
        assert_eq!(
            render_result(&schema, &value),
            Some("ticker: AAPL\ncurrent price (USD): 234".to_string())
        );
    }

    #[test]
    fn render_object_single_prop_renders_bare_value() {
        let schema = json!({
            "type": "object",
            "properties": {"joke": {"type": "string", "description": "the joke"}}
        });
        let value = json!({"joke": "Why did the chicken cross the road?"});
        assert_eq!(
            render_result(&schema, &value),
            Some("Why did the chicken cross the road?".to_string())
        );
    }

    #[test]
    fn render_object_empty_result_returns_none() {
        let schema = json!({
            "type": "object",
            "properties": {"emergency": {"type": "string"}}
        });
        assert_eq!(render_result(&schema, &json!({})), None);
    }

    #[test]
    fn render_object_skips_absent_optional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string"},
                "price": {"type": "number"},
                "change_pct": {"type": "number"}
            }
        });
        // Only `symbol` present; absent optional fields skipped → single line → bare value.
        let value = json!({"symbol": "AAPL"});
        assert_eq!(render_result(&schema, &value), Some("AAPL".to_string()));
    }

    #[test]
    fn render_falls_back_to_key_when_no_description() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            }
        });
        let value = json!({"name": "Ada", "age": 30});
        assert_eq!(
            render_result(&schema, &value),
            Some("name: Ada\nage: 30".to_string())
        );
    }

    #[test]
    fn parse_string_type_takes_raw_input() {
        let spec = ResultSpec::new(json!({"type": "string"})).unwrap();
        assert_eq!(spec.parse("hello").unwrap(), Value::String("hello".to_string()));
    }

    #[test]
    fn parse_number_type_decodes_json() {
        let spec = ResultSpec::new(json!({"type": "number"})).unwrap();
        assert_eq!(spec.parse("42").unwrap(), json!(42));
    }

    #[test]
    fn parse_object_type_decodes_json() {
        let spec = ResultSpec::new(json!({"type": "object"})).unwrap();
        let parsed = spec.parse(r#"{"a":1}"#).unwrap();
        assert_eq!(parsed, json!({"a": 1}));
    }
}
