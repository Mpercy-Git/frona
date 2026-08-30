use serde_json::{Map, Value};

pub struct RequestParams {
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub additional_params: Option<Value>,
}

pub type RequestHook = fn(RequestParams) -> RequestParams;

/// gpt-5/o-series reject `max_tokens` outright; only `max_completion_tokens`
/// is accepted across all current OpenAI models.
pub fn openai(mut p: RequestParams) -> RequestParams {
    if let Some(mt) = p.max_tokens.take() {
        let mut root = take_object(&mut p.additional_params);
        root.entry("max_completion_tokens".to_string())
            .or_insert_with(|| Value::Number(mt.into()));
        p.additional_params = Some(Value::Object(root));
    }
    p
}

/// Groq's API rejects `max_tokens` (not in the Groq completion request struct)
/// and rejects OpenAI-specific params like `reasoning_effort` that leak through
/// `GroqAdditionalParameters.extra`. Move `max_tokens` → `max_completion_tokens`
/// and strip unsupported fields.
pub fn groq(mut p: RequestParams) -> RequestParams {
    if let Some(mt) = p.max_tokens.take() {
        let mut root = take_object(&mut p.additional_params);
        root.insert("max_completion_tokens".to_string(), Value::Number(mt.into()));
        p.additional_params = Some(Value::Object(root));
    }
    // Strip OpenAI-specific params that Groq doesn't understand.
    if let Some(Value::Object(ref mut root)) = p.additional_params {
        root.remove("reasoning_effort");
        root.remove("logprobs");
        root.remove("top_logprobs");
    }
    p
}

/// Ollama silently ignores top-level `max_tokens` — the cap belongs in
/// `options.num_predict`. Rig's Ollama provider doesn't do this rewrite.
pub fn ollama(mut p: RequestParams) -> RequestParams {
    if let Some(mt) = p.max_tokens.take() {
        let mut root = take_object(&mut p.additional_params);
        let mut options = match root.remove("options") {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        options
            .entry("num_predict".to_string())
            .or_insert_with(|| Value::Number(mt.into()));
        root.insert("options".to_string(), Value::Object(options));
        p.additional_params = Some(Value::Object(root));
    }
    p
}

/// Enable Anthropic's automatic prompt caching. rig flattens `additional_params`
/// into the request body, so adding a top-level `cache_control: {"type":"ephemeral"}`
/// is equivalent to rig's `with_automatic_caching()`: the API places a cache
/// breakpoint on the last cacheable block (tools + system + history) and advances
/// it as the conversation grows. This avoids reprocessing the (large, stable)
/// system prompt and tool definitions on every turn and every delegated hop.
///
/// Uses the default 5-minute TTL (no `ttl` field), so no beta header is needed.
/// The API silently skips caching when the prefix is below the model's minimum
/// cacheable length, so it's safe to send unconditionally. Respects a
/// `cache_control` already supplied upstream.
pub fn anthropic(mut p: RequestParams) -> RequestParams {
    let mut root = take_object(&mut p.additional_params);
    root.entry("cache_control".to_string())
        .or_insert_with(|| serde_json::json!({ "type": "ephemeral" }));
    p.additional_params = Some(Value::Object(root));
    p
}

/// OpenRouter takes provider preferences in a top-level `provider` object.
/// The config models the same thing as `provider_routing` so the field doesn't
/// collide with the `#[serde(tag = "provider")]` discriminant on
/// `ProviderModel`, which means the rename has to happen on the way out —
/// otherwise the object ships under a key OpenRouter doesn't read and every
/// routing preference (order, sort, ignore, quantizations) is silently dropped.
///
/// `prompt_caching` is a frona-side toggle consumed when the completion model
/// is built, not an API field, so it is stripped here rather than sent.
pub fn openrouter(mut p: RequestParams) -> RequestParams {
    if let Some(Value::Object(ref mut root)) = p.additional_params {
        root.remove("prompt_caching");
        if let Some(routing) = root.remove("provider_routing") {
            root.insert("provider".to_string(), routing);
        }
        if root.is_empty() {
            p.additional_params = None;
        }
    }
    p
}

fn take_object(slot: &mut Option<Value>) -> Map<String, Value> {
    match slot.take() {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(max_tokens: Option<u64>, additional: Option<Value>) -> RequestParams {
        RequestParams {
            max_tokens,
            temperature: None,
            additional_params: additional,
        }
    }

    #[test]
    fn openai_moves_max_tokens_to_max_completion_tokens() {
        let p = openai(params(Some(64000), None));
        assert!(p.max_tokens.is_none(), "max_tokens should be cleared");
        assert_eq!(
            p.additional_params,
            Some(json!({"max_completion_tokens": 64000})),
        );
    }

    #[test]
    fn openai_merges_with_existing_additional_params() {
        let p = openai(params(
            Some(64000),
            Some(json!({"reasoning_effort": "high"})),
        ));
        assert!(p.max_tokens.is_none());
        assert_eq!(
            p.additional_params,
            Some(json!({
                "reasoning_effort": "high",
                "max_completion_tokens": 64000
            })),
        );
    }

    #[test]
    fn openai_preserves_user_supplied_max_completion_tokens() {
        let p = openai(params(
            Some(8000),
            Some(json!({"max_completion_tokens": 64000})),
        ));
        assert!(p.max_tokens.is_none());
        assert_eq!(
            p.additional_params,
            Some(json!({"max_completion_tokens": 64000})),
        );
    }

    #[test]
    fn openai_skips_when_max_tokens_is_none() {
        let p = openai(params(None, Some(json!({"reasoning_effort": "low"}))));
        assert!(p.max_tokens.is_none());
        assert_eq!(
            p.additional_params,
            Some(json!({"reasoning_effort": "low"}))
        );
    }

    #[test]
    fn ollama_nests_max_tokens_as_num_predict_under_options() {
        let p = ollama(params(Some(8192), None));
        assert!(p.max_tokens.is_none());
        assert_eq!(
            p.additional_params,
            Some(json!({"options": {"num_predict": 8192}})),
        );
    }

    #[test]
    fn ollama_merges_with_existing_options() {
        let p = ollama(params(
            Some(8192),
            Some(json!({"options": {"num_ctx": 32768}, "think": true})),
        ));
        assert!(p.max_tokens.is_none());
        assert_eq!(
            p.additional_params,
            Some(json!({
                "options": {"num_ctx": 32768, "num_predict": 8192},
                "think": true,
            })),
        );
    }

    #[test]
    fn ollama_preserves_user_supplied_num_predict() {
        let p = ollama(params(
            Some(8192),
            Some(json!({"options": {"num_predict": 4096}})),
        ));
        assert!(p.max_tokens.is_none());
        assert_eq!(
            p.additional_params,
            Some(json!({"options": {"num_predict": 4096}})),
        );
    }

    #[test]
    fn anthropic_adds_cache_control_when_absent() {
        let p = anthropic(params(Some(64000), None));
        assert_eq!(
            p.additional_params,
            Some(json!({"cache_control": {"type": "ephemeral"}})),
        );
    }

    #[test]
    fn anthropic_merges_with_existing_additional_params() {
        let p = anthropic(params(
            Some(64000),
            Some(json!({"thinking": {"type": "enabled", "budget_tokens": 16000}})),
        ));
        assert_eq!(
            p.additional_params,
            Some(json!({
                "thinking": {"type": "enabled", "budget_tokens": 16000},
                "cache_control": {"type": "ephemeral"},
            })),
        );
    }

    #[test]
    fn openrouter_renames_provider_routing_to_provider() {
        let p = openrouter(params(
            Some(8192),
            Some(json!({
                "provider_routing": {"order": ["Anthropic"], "sort": "throughput"},
                "top_p": 0.9,
            })),
        ));
        assert_eq!(
            p.additional_params,
            Some(json!({
                "provider": {"order": ["Anthropic"], "sort": "throughput"},
                "top_p": 0.9,
            })),
        );
        // max_tokens is a first-class OpenRouter field, so it stays put.
        assert_eq!(p.max_tokens, Some(8192));
    }

    #[test]
    fn openrouter_strips_the_frona_side_prompt_caching_toggle() {
        let p = openrouter(params(None, Some(json!({"prompt_caching": false}))));
        assert_eq!(
            p.additional_params, None,
            "a body of nothing but the toggle should collapse to no params"
        );
    }

    #[test]
    fn openrouter_leaves_a_body_without_routing_alone() {
        let p = openrouter(params(None, Some(json!({"reasoning_effort": "low"}))));
        assert_eq!(
            p.additional_params,
            Some(json!({"reasoning_effort": "low"}))
        );
    }

    #[test]
    fn anthropic_preserves_user_supplied_cache_control() {
        let p = anthropic(params(
            None,
            Some(json!({"cache_control": {"type": "ephemeral", "ttl": "1h"}})),
        ));
        assert_eq!(
            p.additional_params,
            Some(json!({"cache_control": {"type": "ephemeral", "ttl": "1h"}})),
        );
    }
}
