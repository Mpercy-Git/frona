use std::path::Path;

pub fn detect_content_type(filename: &str) -> &'static str {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "rb" => "text/x-ruby",
        "go" => "text/x-go",
        "java" => "text/x-java",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "cxx" | "hpp" => "text/x-c++",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "yaml" | "yml" => "text/yaml",
        "toml" => "text/toml",
        "sh" | "bash" | "zsh" => "text/x-shellscript",
        "sql" => "text/x-sql",
        "dockerfile" => "text/x-dockerfile",
        // Audio. Anything landing on application/octet-stream won't play in a
        // browser, so the common container extensions are all listed.
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        // Bare .ogg is overwhelmingly audio in practice; .ogv covers the video
        // case below.
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "weba" => "audio/webm",
        // Video.
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "m4v" => "video/x-m4v",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "ogv" => "video/ogg",
        "3gp" => "video/3gpp",
        "mpeg" | "mpg" => "video/mpeg",
        _ => "application/octet-stream",
    }
}

pub fn is_image_content_type(content_type: &str) -> bool {
    content_type.starts_with("image/")
}

pub fn is_text_content_type(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type == "application/json"
        || content_type == "application/xml"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_common_types() {
        assert_eq!(detect_content_type("photo.png"), "image/png");
        assert_eq!(detect_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(detect_content_type("doc.pdf"), "application/pdf");
        assert_eq!(detect_content_type("code.rs"), "text/x-rust");
        assert_eq!(detect_content_type("data.json"), "application/json");
        assert_eq!(detect_content_type("readme.md"), "text/markdown");
        assert_eq!(
            detect_content_type("unknown.xyz"),
            "application/octet-stream"
        );
    }

    #[test]
    fn detect_media_types() {
        // A media file typed as application/octet-stream won't play in the
        // browser, so every common container needs an entry.
        assert_eq!(detect_content_type("song.mp3"), "audio/mpeg");
        assert_eq!(detect_content_type("voice.m4a"), "audio/mp4");
        assert_eq!(detect_content_type("note.ogg"), "audio/ogg");
        assert_eq!(detect_content_type("note.opus"), "audio/opus");
        assert_eq!(detect_content_type("track.flac"), "audio/flac");
        assert_eq!(detect_content_type("clip.mp4"), "video/mp4");
        assert_eq!(detect_content_type("clip.MOV"), "video/quicktime");
        assert_eq!(detect_content_type("clip.mkv"), "video/x-matroska");
        assert_eq!(detect_content_type("clip.ogv"), "video/ogg");
    }

    #[test]
    fn image_and_text_detection() {
        assert!(is_image_content_type("image/png"));
        assert!(is_image_content_type("image/jpeg"));
        assert!(!is_image_content_type("text/plain"));

        assert!(is_text_content_type("text/plain"));
        assert!(is_text_content_type("text/x-rust"));
        assert!(is_text_content_type("application/json"));
        assert!(!is_text_content_type("image/png"));
    }
}
