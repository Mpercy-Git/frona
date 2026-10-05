//! Speech-to-text for voice notes.
//!
//! A voice note reaches the model as a file path it has no way to listen to, so
//! the agent answers a recording it never heard. Audio attachments are sent to an
//! OpenAI-compatible `/audio/transcriptions` endpoint once, when the message is
//! saved, and the transcript is stored on the attachment. History rebuilds then
//! read the stored text instead of paying for the same recording every turn.
//!
//! The endpoint is the de-facto standard: OpenAI, Groq, and local Whisper servers
//! (faster-whisper-server, whisper.cpp's server, LocalAI, vLLM) all accept it, so
//! one client covers hosted and self-hosted alike. Credentials come from the
//! provider entry `voice.transcription_provider` names, so nothing is configured
//! twice.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::core::config::{ModelProviderConfig, VoiceConfig};
use crate::core::error::AppError;
use crate::core::handle::Handle;
use crate::storage::Attachment;

/// OpenAI and Groq both reject uploads over 25 MB.
pub const MAX_AUDIO_BYTES: u64 = 25 * 1024 * 1024;

/// A minute of speech transcribes in a few seconds; this only stops a stuck
/// endpoint from holding the message back forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Picked, in order, when `voice.transcription_provider` is unset: the hosted
/// providers whose transcription endpoint sits at a known address.
const AUTO_PROVIDERS: &[&str] = &["openai", "groq"];

#[derive(Debug, Clone)]
pub struct TranscriptionService {
    client: reqwest::Client,
    provider: String,
    endpoint: String,
    api_key: Option<String>,
    model: String,
    language: Option<String>,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

impl TranscriptionService {
    /// `None` when transcription is switched off (`transcription_provider:
    /// none`), or when no configured provider can serve it. Neither is an error:
    /// voice notes then reach the model as files, exactly as before.
    pub fn from_config(
        voice: &VoiceConfig,
        providers: &HashMap<Handle, ModelProviderConfig>,
    ) -> Option<Self> {
        let requested = voice
            .transcription_provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if requested.is_some_and(|p| p.eq_ignore_ascii_case("none")) {
            return None;
        }

        let (handle, config) = match requested {
            Some(name) => {
                let Some(entry) = providers.iter().find(|(handle, _)| handle.as_str() == name)
                else {
                    tracing::warn!(
                        provider = name,
                        "voice.transcription_provider names a provider that isn't configured; voice notes won't be transcribed"
                    );
                    return None;
                };
                entry
            }
            None => {
                let mut candidates: Vec<_> = providers
                    .iter()
                    .filter(|(handle, config)| {
                        config.enabled
                            && resolved_api_key(config).is_some()
                            && AUTO_PROVIDERS.contains(&brand(handle, config))
                    })
                    .collect();
                candidates.sort_by_key(|(handle, config)| {
                    AUTO_PROVIDERS
                        .iter()
                        .position(|p| *p == brand(handle, config))
                });
                candidates.into_iter().next()?
            }
        };

        if !config.enabled {
            tracing::warn!(provider = %handle.as_str(), "transcription provider is disabled; voice notes won't be transcribed");
            return None;
        }
        let brand = brand(handle, config);
        let Some(base_url) = config
            .base_url
            .as_deref()
            .map(crate::core::config::expand_env_vars)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| default_base_url(brand).map(String::from))
        else {
            tracing::warn!(
                provider = %handle.as_str(),
                brand,
                "transcription provider has no base_url and no known default; voice notes won't be transcribed"
            );
            return None;
        };

        let model = voice
            .transcription_model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| default_model(brand).to_string());

        let service = Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            provider: handle.as_str().to_string(),
            endpoint: format!("{}/audio/transcriptions", base_url.trim_end_matches('/')),
            api_key: resolved_api_key(config),
            model,
            language: voice
                .transcription_language
                .clone()
                .filter(|l| !l.trim().is_empty()),
        };
        tracing::info!(
            provider = %service.provider,
            model = %service.model,
            "Voice note transcription enabled"
        );
        Some(service)
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn transcribe(
        &self,
        audio: Vec<u8>,
        filename: &str,
        content_type: &str,
    ) -> Result<String, AppError> {
        let part = reqwest::multipart::Part::bytes(audio)
            .file_name(upload_filename(filename, content_type))
            .mime_str(content_type)
            .map_err(|e| AppError::Validation(format!("unusable audio content type: {e}")))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", self.model.clone())
            .text("response_format", "json");
        if let Some(language) = &self.language {
            form = form.text("language", language.clone());
        }

        let mut request = self.client.post(&self.endpoint).multipart(form);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|e| {
            AppError::Internal(format!(
                "transcription request to {} failed: {e}",
                self.provider
            ))
        })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(300).collect();
            return Err(AppError::Internal(format!(
                "transcription by {} failed with {status}: {body}",
                self.provider
            )));
        }
        let parsed: TranscriptionResponse = response.json().await.map_err(|e| {
            AppError::Internal(format!(
                "transcription by {} returned an unreadable response: {e}",
                self.provider
            ))
        })?;
        Ok(parsed.text.trim().to_string())
    }
}

/// Whether `attachment` is a recording still waiting for its transcript.
pub fn needs_transcript(attachment: &Attachment) -> bool {
    attachment
        .content_type
        .to_ascii_lowercase()
        .starts_with("audio/")
        && attachment.transcript.is_none()
}

/// The name the upload goes out under. Hosted endpoints pick the decoder from
/// the extension and accept only the common ones, so a browser recording saved
/// as `.weba` (the extension frona maps to `audio/webm`) would be refused for its
/// name alone. Known audio types get the extension the endpoint expects.
fn upload_filename(filename: &str, content_type: &str) -> String {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ext = match essence.as_str() {
        "audio/webm" => "webm",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/flac" => "flac",
        _ => return filename.to_string(),
    };
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("voice-note");
    format!("{stem}.{ext}")
}

fn brand<'a>(handle: &'a Handle, config: &'a ModelProviderConfig) -> &'a str {
    config.provider.as_deref().unwrap_or(handle.as_str())
}

fn resolved_api_key(config: &ModelProviderConfig) -> Option<String> {
    config
        .api_key
        .as_deref()
        .map(crate::core::config::expand_env_vars)
        .filter(|k| !k.trim().is_empty())
}

fn default_base_url(brand: &str) -> Option<&'static str> {
    match brand {
        "openai" => Some("https://api.openai.com/v1"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        _ => None,
    }
}

fn default_model(brand: &str) -> &'static str {
    match brand {
        "openai" => "gpt-4o-mini-transcribe",
        "groq" => "whisper-large-v3-turbo",
        // The name local Whisper servers conventionally answer to.
        _ => "whisper-1",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn providers(entries: &[(&str, ModelProviderConfig)]) -> HashMap<Handle, ModelProviderConfig> {
        entries
            .iter()
            .map(|(name, config)| (Handle::try_new(name).unwrap(), config.clone()))
            .collect()
    }

    fn keyed(base_url: Option<&str>) -> ModelProviderConfig {
        ModelProviderConfig {
            api_key: Some("sk-test".into()),
            base_url: base_url.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn picks_openai_before_groq_when_unset() {
        let service = TranscriptionService::from_config(
            &VoiceConfig::default(),
            &providers(&[("groq", keyed(None)), ("openai", keyed(None))]),
        )
        .unwrap();
        assert_eq!(service.provider(), "openai");
        assert_eq!(service.model(), "gpt-4o-mini-transcribe");
        assert_eq!(
            service.endpoint,
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn unset_with_no_capable_provider_is_off() {
        assert!(
            TranscriptionService::from_config(
                &VoiceConfig::default(),
                &providers(&[("anthropic", keyed(None))]),
            )
            .is_none()
        );
    }

    #[test]
    fn none_switches_it_off() {
        let voice = VoiceConfig {
            transcription_provider: Some("none".into()),
            ..Default::default()
        };
        assert!(
            TranscriptionService::from_config(&voice, &providers(&[("openai", keyed(None))]))
                .is_none()
        );
    }

    #[test]
    fn named_generic_provider_uses_its_base_url_without_a_key() {
        let voice = VoiceConfig {
            transcription_provider: Some("whisper".into()),
            ..Default::default()
        };
        let local = ModelProviderConfig {
            provider: Some("generic".into()),
            base_url: Some("http://whisper:8000/v1/".into()),
            ..Default::default()
        };
        let service =
            TranscriptionService::from_config(&voice, &providers(&[("whisper", local)])).unwrap();
        assert_eq!(
            service.endpoint,
            "http://whisper:8000/v1/audio/transcriptions"
        );
        assert_eq!(service.model(), "whisper-1");
        assert!(service.api_key.is_none());
    }

    #[test]
    fn named_provider_without_base_url_or_default_is_off() {
        let voice = VoiceConfig {
            transcription_provider: Some("anthropic".into()),
            ..Default::default()
        };
        assert!(
            TranscriptionService::from_config(&voice, &providers(&[("anthropic", keyed(None))]))
                .is_none()
        );
    }

    #[test]
    fn only_untranscribed_audio_needs_a_transcript() {
        let mut att = Attachment {
            filename: "note.ogg".into(),
            content_type: "audio/ogg; codecs=opus".into(),
            size_bytes: 10,
            owner: "user:u".into(),
            path: "note.ogg".into(),
            url: None,
            transcript: None,
        };
        assert!(needs_transcript(&att));
        att.transcript = Some("hello".into());
        assert!(!needs_transcript(&att));
        att.transcript = None;
        att.content_type = "image/png".into();
        assert!(!needs_transcript(&att));
    }

    #[test]
    fn upload_name_carries_an_extension_the_endpoint_accepts() {
        assert_eq!(
            upload_filename("voice-note.weba", "audio/webm"),
            "voice-note.webm"
        );
        assert_eq!(
            upload_filename("voice.oga", "audio/ogg; codecs=opus"),
            "voice.ogg"
        );
        assert_eq!(upload_filename("memo.m4a", "audio/mp4"), "memo.m4a");
        assert_eq!(upload_filename("odd.bin", "audio/x-unknown"), "odd.bin");
    }

    #[tokio::test]
    async fn sends_multipart_and_reads_text() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"text": "  remind me at five  "})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let voice = VoiceConfig {
            transcription_provider: Some("local".into()),
            transcription_language: Some("en".into()),
            ..Default::default()
        };
        let service = TranscriptionService::from_config(
            &voice,
            &providers(&[("local", keyed(Some(&format!("{}/v1", server.uri()))))]),
        )
        .unwrap();
        let text = service
            .transcribe(b"OggS fake".to_vec(), "note.ogg", "audio/ogg")
            .await
            .unwrap();
        assert_eq!(text, "remind me at five");

        let received = &server.received_requests().await.unwrap()[0];
        let body = String::from_utf8_lossy(&received.body);
        assert!(body.contains("name=\"model\""));
        assert!(body.contains("whisper-1"));
        assert!(body.contains("name=\"language\""));
        assert!(body.contains("filename=\"note.ogg\""));
    }

    #[tokio::test]
    async fn surfaces_provider_errors() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("unsupported format"))
            .mount(&server)
            .await;
        let voice = VoiceConfig {
            transcription_provider: Some("local".into()),
            ..Default::default()
        };
        let service = TranscriptionService::from_config(
            &voice,
            &providers(&[("local", keyed(Some(&server.uri())))]),
        )
        .unwrap();
        let err = service
            .transcribe(b"x".to_vec(), "a.webm", "audio/webm")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("400"), "{err}");
        assert!(err.contains("unsupported format"), "{err}");
    }
}
