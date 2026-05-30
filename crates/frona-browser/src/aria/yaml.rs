pub fn yaml_scalar(s: &str) -> String {
    serde_yaml::to_string(s)
        .expect("string serialization to YAML never fails")
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_input_as_value() {
        for s in [
            "", "simple", "hello-world", "foo_bar", " text", "text ", "\t",
            "-foo", "&anchor", "*alias", "?key", "!tag", "|literal", ">folded",
            "@x", "#hash", "%directive", "[array", "{flow",
            "foo: bar", "foo:", "foo #comment", "a\nb", "a\rb",
            "foo{bar", "foo}bar", "foo`bar", "foo\"bar", "foo'bar", "foo\\bar",
            "y", "n", "yes", "no", "true", "false", "on", "off", "null",
            "0", "1", "123", "-1", "1.5", "1e10", "inf", "nan",
            "héllo", "日本語", "🎉",
            "hello\nworld", "quote\"here", "back\\slash", "tab\there",
            "\x00", "\x07", "\x1B",
        ] {
            let escaped = yaml_scalar(s);
            let doc = format!("v: {escaped}");
            let parsed: serde_yaml::Value = serde_yaml::from_str(&doc)
                .unwrap_or_else(|e| panic!("invalid YAML for {s:?} → {escaped:?}: {e}"));
            let v = parsed.get("v").and_then(|v| v.as_str()).unwrap_or_else(|| {
                panic!("input {s:?} → {escaped:?} did not round-trip as string: got {parsed:?}")
            });
            assert_eq!(v, s, "round-trip mismatch for {s:?}: escaped={escaped:?}");
        }
    }

    #[test]
    fn round_trip_preserves_input_as_key() {
        for s in [
            "simple", "it's", "-start", "true", "foo: bar", "héllo", "🎉", "yes",
        ] {
            let escaped = yaml_scalar(s);
            let doc = format!("{escaped}: 0");
            let parsed: serde_yaml::Value = serde_yaml::from_str(&doc)
                .unwrap_or_else(|e| panic!("invalid YAML for key {s:?} → {escaped:?}: {e}"));
            let mapping = parsed.as_mapping().expect("expected mapping");
            let key = mapping.keys().next().expect("expected one key");
            let k = key.as_str().unwrap_or_else(|| {
                panic!("key {s:?} → {escaped:?} did not parse as string: got {key:?}")
            });
            assert_eq!(k, s, "key round-trip mismatch for {s:?}: escaped={escaped:?}");
        }
    }

    #[test]
    fn plain_scalars_pass_through_unquoted() {
        for s in ["simple", "hello-world", "foo_bar", "abc123"] {
            let out = yaml_scalar(s);
            assert_eq!(out, s, "expected {s:?} to round-trip unquoted, got {out:?}");
        }
    }
}
