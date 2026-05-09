use std::collections::BTreeMap;

/// `Value::Null` removes the key; other values upsert.
pub fn apply_metadata_patch(
    target: &mut BTreeMap<String, serde_json::Value>,
    patch: BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in patch {
        if value.is_null() {
            target.remove(&key);
        } else {
            target.insert(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upserts_set_keys() {
        let mut md = BTreeMap::new();
        md.insert("a".into(), json!(1));
        let mut patch = BTreeMap::new();
        patch.insert("a".into(), json!(2));
        patch.insert("b".into(), json!("hello"));
        apply_metadata_patch(&mut md, patch);
        assert_eq!(md.get("a"), Some(&json!(2)));
        assert_eq!(md.get("b"), Some(&json!("hello")));
    }

    #[test]
    fn null_value_removes_key() {
        let mut md = BTreeMap::new();
        md.insert("a".into(), json!(1));
        md.insert("b".into(), json!(2));
        let mut patch = BTreeMap::new();
        patch.insert("a".into(), serde_json::Value::Null);
        apply_metadata_patch(&mut md, patch);
        assert!(!md.contains_key("a"));
        assert_eq!(md.get("b"), Some(&json!(2)));
    }

    #[test]
    fn unmentioned_keys_untouched() {
        let mut md = BTreeMap::new();
        md.insert("keep".into(), json!("me"));
        let patch = BTreeMap::new();
        apply_metadata_patch(&mut md, patch);
        assert_eq!(md.get("keep"), Some(&json!("me")));
    }

    #[test]
    fn empty_target_with_patch() {
        let mut md = BTreeMap::new();
        let mut patch = BTreeMap::new();
        patch.insert("new".into(), json!(42));
        patch.insert("ignored".into(), serde_json::Value::Null);
        apply_metadata_patch(&mut md, patch);
        assert_eq!(md.get("new"), Some(&json!(42)));
        assert!(!md.contains_key("ignored"));
    }
}
