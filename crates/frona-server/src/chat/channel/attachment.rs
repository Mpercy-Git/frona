//! Every URL these helpers produce eventually hits `/api/files/{owner}/{path}`
//! and is authenticated by `FileAuth` (session cookie or Bearer). No URL
//! carries a presign token; auth lives at the redirect target.

use crate::core::error::AppError;
use crate::storage::Attachment;

use super::models::ChannelCtx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    Audio,
    Video,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    /// Telegram, Discord, Slack, WhatsApp Cloud. The URL is hidden behind
    /// a button label, so length doesn't matter; we use the long descriptive
    /// form and skip the DB row.
    Button,
    /// SMS, Signal, WhatsApp User. The URL is rendered as visible text in the
    /// message body, so we mint a short id (one `share` row per attachment).
    Inline,
}

pub fn classify(att: &Attachment) -> AttachmentKind {
    let ct = att.content_type.to_ascii_lowercase();
    if crate::storage::is_image_content_type(&ct) {
        AttachmentKind::Image
    } else if ct.starts_with("audio/") {
        AttachmentKind::Audio
    } else if ct.starts_with("video/") {
        AttachmentKind::Video
    } else {
        AttachmentKind::Document
    }
}

pub fn is_media(kind: AttachmentKind) -> bool {
    !matches!(kind, AttachmentKind::Document)
}

/// `"📄 report.md — https://app.host/s/8Dbcv_bu"`
pub fn inline_list_line(att: &Attachment, url: &str) -> String {
    format!("{} {} — {url}", icon_for(att), att.filename)
}

/// Lengths of `attachments` and `urls` must match; if not, the shorter wins.
pub fn inline_list(attachments: &[Attachment], urls: &[String]) -> String {
    attachments
        .iter()
        .zip(urls.iter())
        .map(|(att, url)| inline_list_line(att, url))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn button_label(att: &Attachment) -> String {
    format!("{} Open {}", icon_for(att), att.filename)
}

fn icon_for(att: &Attachment) -> &'static str {
    match classify(att) {
        AttachmentKind::Image => "🖼️",
        AttachmentKind::Audio => "🎵",
        AttachmentKind::Video => "🎬",
        AttachmentKind::Document => "📄",
    }
}

/// Whitelist of content types that browsers would otherwise display as raw
/// text — markdown, source code, structured data. Everything else (media,
/// PDFs, plain text, binary) renders natively at the raw URL.
pub fn is_previewable(att: &Attachment) -> bool {
    let ct = att.content_type.to_ascii_lowercase();
    ct == "text/markdown"
        || ct.starts_with("text/x-")
        || ct == "text/yaml"
        || ct == "text/toml"
        || ct == "application/json"
        || ct == "application/xml"
}

/// | `mode`   | `is_previewable` | Returns                                    | DB row? |
/// |----------|------------------|--------------------------------------------|---------|
/// | `Inline` | true             | `{base}/p/{8-char-id}`                     | yes     |
/// | `Inline` | false            | `{base}/s/{8-char-id}`                     | yes     |
/// | `Button` | true             | `{base}/p/{owner_segment}/{path}`          | no      |
/// | `Button` | false            | `{base}/api/files/{owner_segment}/{path}`  | no      |
pub async fn outbound_url(
    att: &Attachment,
    ctx: &ChannelCtx,
    mode: ChannelMode,
) -> Result<String, AppError> {
    let base = ctx.base_url.trim_end_matches('/');
    let preview = is_previewable(att);
    match mode {
        ChannelMode::Inline => {
            let id = ctx
                .share_service
                .issue_file(&att.owner, &att.path, &ctx.channel.user_id, ctx.share_ttl_secs)
                .await?;
            if preview {
                Ok(format!("{base}/p/{id}"))
            } else {
                Ok(format!("{base}/s/{id}"))
            }
        }
        ChannelMode::Button => {
            let segment = build_owner_segment(att, ctx).await?;
            if preview {
                Ok(format!("{base}/p/{segment}"))
            } else {
                Ok(format!("{base}/api/files/{segment}"))
            }
        }
    }
}

async fn build_owner_segment(att: &Attachment, ctx: &ChannelCtx) -> Result<String, AppError> {
    let needs_user_handle = att.owner.starts_with("user:");
    let user_handle_storage;
    let user_handle_ref: Option<&str> = if needs_user_handle {
        user_handle_storage = ctx.user_service.handle_of(&ctx.channel.user_id).await?;
        Some(user_handle_storage.as_ref())
    } else {
        None
    };
    crate::storage::attachment_url_segment(&att.owner, &att.path, user_handle_ref).ok_or_else(|| {
        AppError::Validation(format!(
            "cannot build URL segment for owner={:?} path={:?}",
            att.owner, att.path
        ))
    })
}

/// Handles both `user:{handle}` and `agent:{handle}` owners.
pub async fn read_attachment_bytes(
    att: &Attachment,
    ctx: &ChannelCtx,
) -> Result<Vec<u8>, AppError> {
    let path_str = &att.path;
    let owner = &att.owner;
    let abs = if let Some(owner_handle_str) = owner.strip_prefix("user:") {
        let owner_handle = crate::core::Handle::try_new(owner_handle_str).map_err(|e| {
            AppError::Validation(format!("invalid owner handle in {owner}: {e}"))
        })?;
        let workspace = ctx.storage_service.user_workspace(&owner_handle);
        workspace
            .resolve_path(path_str)
            .ok_or_else(|| AppError::NotFound(format!("attachment {path_str} not in user workspace")))?
    } else if let Some(agent_handle_str) = owner.strip_prefix("agent:") {
        let agent_handle = crate::core::Handle::try_new(agent_handle_str).map_err(|e| {
            AppError::Validation(format!("invalid agent handle in {owner}: {e}"))
        })?;
        let user_handle = ctx.user_service.handle_of(&ctx.channel.user_id).await?;
        let base = ctx.storage_service.agent_workspace_path(&user_handle, &agent_handle);
        if path_str.contains("..") {
            return Err(AppError::Validation("Path traversal not allowed".into()));
        }
        let abs = base.join(path_str);
        if !abs.exists() {
            return Err(AppError::NotFound(format!(
                "attachment {path_str} not in agent workspace"
            )));
        }
        abs
    } else {
        return Err(AppError::Validation(format!(
            "unsupported attachment owner: {owner}"
        )));
    };
    std::fs::read(&abs)
        .map_err(|e| AppError::Internal(format!("read attachment {path_str}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(filename: &str, content_type: &str, owner: &str) -> Attachment {
        Attachment {
            filename: filename.into(),
            content_type: content_type.into(),
            size_bytes: 0,
            owner: owner.into(),
            path: filename.into(),
            url: None,
        }
    }

    #[test]
    fn classify_matrix() {
        assert_eq!(classify(&att("a.png", "image/png", "user:x")), AttachmentKind::Image);
        assert_eq!(classify(&att("a.jpg", "IMAGE/JPEG", "user:x")), AttachmentKind::Image);
        assert_eq!(classify(&att("a.mp3", "audio/mpeg", "user:x")), AttachmentKind::Audio);
        assert_eq!(classify(&att("a.mp4", "video/mp4", "user:x")), AttachmentKind::Video);
        assert_eq!(classify(&att("a.md", "text/markdown", "user:x")), AttachmentKind::Document);
        assert_eq!(classify(&att("a.pdf", "application/pdf", "user:x")), AttachmentKind::Document);
        assert_eq!(classify(&att("a.csv", "text/csv", "user:x")), AttachmentKind::Document);
        assert_eq!(classify(&att("a.zip", "application/zip", "user:x")), AttachmentKind::Document);
    }

    #[test]
    fn is_media_only_media_kinds() {
        assert!(is_media(AttachmentKind::Image));
        assert!(is_media(AttachmentKind::Audio));
        assert!(is_media(AttachmentKind::Video));
        assert!(!is_media(AttachmentKind::Document));
    }

    #[test]
    fn inline_list_line_format() {
        let line = inline_list_line(
            &att("report.md", "text/markdown", "agent:researcher"),
            "https://app.host/s/abc12345",
        );
        assert_eq!(line, "📄 report.md — https://app.host/s/abc12345");
    }

    #[test]
    fn inline_list_uses_kind_icon() {
        let line = inline_list_line(
            &att("chart.png", "image/png", "agent:researcher"),
            "https://app.host/s/xyz",
        );
        assert!(line.starts_with("🖼️"));
    }

    #[test]
    fn button_label_includes_filename() {
        let label = button_label(&att("data.csv", "text/csv", "agent:researcher"));
        assert_eq!(label, "📄 Open data.csv");
    }

    #[test]
    fn is_previewable_matrix() {
        assert!(is_previewable(&att("a.md", "text/markdown", "agent:r")));
        assert!(is_previewable(&att("a.rs", "text/x-rust", "agent:r")));
        assert!(is_previewable(&att("a.py", "text/x-python", "agent:r")));
        assert!(is_previewable(&att("a.go", "text/x-go", "agent:r")));
        assert!(is_previewable(&att("a.sh", "text/x-shellscript", "agent:r")));
        assert!(is_previewable(&att("a.json", "application/json", "agent:r")));
        assert!(is_previewable(&att("a.xml", "application/xml", "agent:r")));
        assert!(is_previewable(&att("a.yaml", "text/yaml", "agent:r")));
        assert!(is_previewable(&att("a.toml", "text/toml", "agent:r")));
        assert!(is_previewable(&att("a.md", "TEXT/MARKDOWN", "agent:r")));

        // text/html is excluded deliberately: rendering attacker-controlled
        // HTML inside our preview page would run scripts in the app's origin.
        assert!(!is_previewable(&att("a.html", "text/html", "agent:r")));
        assert!(!is_previewable(&att("a.png", "image/png", "agent:r")));
        assert!(!is_previewable(&att("a.jpg", "image/jpeg", "agent:r")));
        assert!(!is_previewable(&att("a.pdf", "application/pdf", "agent:r")));
        assert!(!is_previewable(&att("a.mp3", "audio/mpeg", "agent:r")));
        assert!(!is_previewable(&att("a.mp4", "video/mp4", "agent:r")));
        assert!(!is_previewable(&att("a.txt", "text/plain", "agent:r")));
        assert!(!is_previewable(&att("a.log", "text/plain", "agent:r")));
        assert!(!is_previewable(&att("a.css", "text/css", "agent:r")));
        assert!(!is_previewable(&att("a.csv", "text/csv", "agent:r")));
        assert!(!is_previewable(&att("a.zip", "application/zip", "agent:r")));
        assert!(!is_previewable(&att("a.bin", "application/octet-stream", "agent:r")));
    }

    #[test]
    fn inline_list_joins_with_newlines() {
        let atts = vec![
            att("a.md", "text/markdown", "agent:x"),
            att("b.csv", "text/csv", "agent:x"),
        ];
        let urls = vec!["url1".to_string(), "url2".to_string()];
        let list = inline_list(&atts, &urls);
        assert_eq!(list, "📄 a.md — url1\n📄 b.csv — url2");
    }
}
