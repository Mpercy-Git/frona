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

/// Complex schemas must include a top-level required `summary` string
/// property. The user-facing renderer surfaces only that field for complex
/// shapes, so requiring it guarantees the result bubble is never empty.
pub fn has_renderable_summary_field(schema: &Value) -> bool {
    let Some(props) = schema.get("properties").and_then(|v| v.as_object()) else {
        return false;
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    if !required.contains(&"summary") {
        return false;
    }
    let Some(prop) = props.get("summary") else { return false };
    prop.get("type").and_then(|v| v.as_str()) == Some("string")
        || prop
            .get("type")
            .and_then(|v| v.as_array())
            .is_some_and(|arr| arr.iter().any(|t| t.as_str() == Some("string")))
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
