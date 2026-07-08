pub(crate) fn convert_html_to_markdown(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }
    html2md::parse_html(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_html() {
        assert_eq!(convert_html_to_markdown(""), "");
    }

    #[test]
    fn test_simple_heading() {
        let md = convert_html_to_markdown("<h1>Test Title</h1>");
        assert!(md.contains("Test Title"));
    }

    #[test]
    fn test_paragraph() {
        let md = convert_html_to_markdown("<p>This is a paragraph.</p>");
        assert!(md.contains("This is a paragraph"));
    }

    #[test]
    fn test_link() {
        let md = convert_html_to_markdown(r#"<a href="https://example.com">Example</a>"#);
        assert!(md.contains("[Example]"));
        assert!(md.contains("https://example.com"));
    }
}
