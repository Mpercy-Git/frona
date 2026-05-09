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
        let target = if matches!(
            self.schema.get("type").and_then(|v| v.as_str()),
            Some("string")
        ) {
            Value::String(result.to_string())
        } else {
            serde_json::from_str(result).map_err(|e| {
                format!("result must be a JSON-encoded value matching the schema: {e}")
            })?
        };
        self.compiled.validate(&target).map_err(|errors| {
            errors
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
}

pub fn validate_schema_doc(schema: &Value) -> Result<(), String> {
    ResultSpec::new(schema.clone()).map(|_| ())
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
}
