use std::collections::HashMap;
use std::sync::Arc;

use minijinja::value::{Enumerator, Object, ObjectRepr, Value};
use minijinja::{Environment, UndefinedBehavior};

use super::error::AppError;

#[derive(Debug)]
struct CaseInsensitiveContext {
    data: HashMap<String, Value>,
}

impl Object for CaseInsensitiveContext {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Map
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        let key_str = key.as_str()?.to_lowercase();
        self.data.get(&key_str).cloned()
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        let keys: Vec<Value> = self.data.keys().map(|k| Value::from(k.clone())).collect();
        Enumerator::Values(keys)
    }
}

pub fn render_template(template: &str, vars: &[(&str, &str)]) -> Result<String, AppError> {
    if vars.is_empty() && !template.contains("{{") {
        return Ok(template.to_string());
    }

    let data: HashMap<String, Value> = vars
        .iter()
        .map(|(k, v)| (k.to_lowercase(), Value::from(*v)))
        .collect();

    let ctx = CaseInsensitiveContext { data };

    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);

    env.render_str(template, Value::from_object(ctx))
        .map_err(|e| AppError::Internal(format!("Template rendering failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_simple_variable() {
        let result = render_template("Hello {{name}}!", &[("name", "World")]).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn case_insensitive_key_side() {
        let result =
            render_template("Hello {{agent_name}}!", &[("Agent_Name", "Bot")]).unwrap();
        assert_eq!(result, "Hello Bot!");
    }

    #[test]
    fn case_insensitive_template_side() {
        let result = render_template("Hello {{NAME}}!", &[("name", "World")]).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn missing_variable_returns_error() {
        let result = render_template("Hello {{missing}}!", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn empty_vars_no_placeholders_returns_unchanged() {
        let input = "No placeholders here.";
        let result = render_template(input, &[]).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn multiple_variables() {
        let result = render_template(
            "{{greeting}}, {{name}}! You are {{role}}.",
            &[("greeting", "Hi"), ("name", "Alice"), ("role", "admin")],
        )
        .unwrap();
        assert_eq!(result, "Hi, Alice! You are admin.");
    }

    #[test]
    fn same_variable_used_twice() {
        let result =
            render_template("{{x}} and {{x}}", &[("x", "hello")]).unwrap();
        assert_eq!(result, "hello and hello");
    }
}
