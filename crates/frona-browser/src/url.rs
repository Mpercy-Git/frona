pub(crate) fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();

    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("file://")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("chrome://")
        || trimmed.starts_with("chrome-extension://")
    {
        return trimmed.to_string();
    }

    if trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("../") {
        return trimmed.to_string();
    }

    if trimmed.starts_with("localhost") || trimmed.starts_with("127.0.0.1") {
        return format!("http://{trimmed}");
    }

    if trimmed.contains('.') {
        return format!("https://{trimmed}");
    }

    format!("https://www.{trimmed}.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_url_complete() {
        assert_eq!(normalize_url("https://example.com"), "https://example.com");
        assert_eq!(normalize_url("http://example.com"), "http://example.com");
        assert_eq!(
            normalize_url("https://example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn test_normalize_url_missing_protocol() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(
            normalize_url("example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn test_normalize_url_partial_domain() {
        assert_eq!(normalize_url("google"), "https://www.google.com");
    }

    #[test]
    fn test_normalize_url_localhost() {
        assert_eq!(normalize_url("localhost"), "http://localhost");
        assert_eq!(normalize_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(normalize_url("127.0.0.1"), "http://127.0.0.1");
    }

    #[test]
    fn test_normalize_url_special_protocols() {
        assert_eq!(normalize_url("about:blank"), "about:blank");
        assert_eq!(normalize_url("file:///x"), "file:///x");
        assert_eq!(
            normalize_url("data:text/html,<h1>Test</h1>"),
            "data:text/html,<h1>Test</h1>"
        );
    }

    #[test]
    fn test_normalize_url_whitespace() {
        assert_eq!(normalize_url("  example.com  "), "https://example.com");
    }
}
