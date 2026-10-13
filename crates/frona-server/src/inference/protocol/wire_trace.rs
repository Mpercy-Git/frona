//! Opt-in capture of final serialized provider requests, including release builds.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static WIRE_CONFIG: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let directory = PathBuf::from(
        std::env::var("FRONA_LLM_WIRE_TRACE")
            .ok()
            .filter(|value| !value.trim().is_empty())?,
    );
    if let Err(error) = std::fs::create_dir_all(&directory) {
        tracing::warn!(error = %error, "Wire trace directory unusable");
        return None;
    }
    tracing::info!(dir = %directory.display(), "Final provider request tracing enabled");
    Some(directory)
});

static WIRE_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Capture the final JSON body, after SDK serialization and parameter merging.
/// Opt-in in both debug and release builds. Never records headers or modifies
/// the outgoing request. Inline data URLs are omitted; prompt text is retained.
pub(crate) fn record_wire_request(body: &serde_json::Value) {
    let Some(directory) = WIRE_CONFIG.as_ref() else {
        return;
    };
    if let Err(error) = write_wire_request(directory, body) {
        tracing::warn!(error = %error, "Cannot write final provider request trace");
    }
}

fn redact_data_urls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                redact_data_urls(item);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                if matches!(
                    key.as_str(),
                    "url" | "image_url" | "file_data" | "audio_url" | "video_url"
                ) && let serde_json::Value::String(text) = value
                    && text.starts_with("data:")
                    && let Some((header, payload)) = text.split_once(',')
                {
                    *text = format!("{header},[omitted {} bytes]", payload.len());
                    continue;
                }
                redact_data_urls(value);
            }
        }
        _ => {}
    }
}

fn write_wire_request(
    directory: &std::path::Path,
    body: &serde_json::Value,
) -> std::io::Result<()> {
    use std::io::Write;

    let mut captured = body.clone();
    redact_data_urls(&mut captured);
    let seq = WIRE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let name = format!(
        "{}-{seq:04}-request.json",
        chrono::Utc::now().timestamp_micros()
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(directory.join(name))?;
    let bytes = serde_json::to_vec_pretty(&captured)?;
    file.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn wire_trace_keeps_text_history_and_routing_without_inline_image_data() {
        let directory = tempfile::tempdir().unwrap();
        let body = serde_json::json!({
            "model": "test-model", "stream": true,
            "provider": {"order": ["test-provider"]},
            "plugins": [{"id": "context-compression", "enabled": false}],
            "messages": [
                {"role": "system", "content": "Follow the user's request."},
                {"role": "user", "content": "data:text/plain,keep this exact user text"},
                {"role": "assistant", "content": "Earlier answer"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Identify the object and its colour."},
                    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,abcd"}}
                ]}
            ]
        });
        let original = body.clone();
        super::write_wire_request(directory.path(), &body).unwrap();
        super::write_wire_request(directory.path(), &body).unwrap();
        let files = std::fs::read_dir(directory.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(files.len(), 2, "each attempt gets its own trace");
        for entry in files {
            let traced: serde_json::Value =
                serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            let mut expected = original.clone();
            expected["messages"][3]["content"][1]["image_url"]["url"] =
                serde_json::json!("data:image/jpeg;base64,[omitted 4 bytes]");
            assert_eq!(traced, expected);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    entry.metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        assert_eq!(body, original, "tracing must not change provider input");
    }

    #[test]
    fn wire_trace_io_failure_does_not_change_the_request() {
        let directory = tempfile::tempdir().unwrap();
        let body = serde_json::json!({"messages": [{"role": "user", "content": "Hello"}]});
        let original = body.clone();
        assert!(super::write_wire_request(&directory.path().join("missing"), &body).is_err());
        assert_eq!(body, original);
    }
}
