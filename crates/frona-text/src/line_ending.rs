#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn detect(text: &str) -> Self {
        let crlf = text.find("\r\n");
        let lf = text.find('\n');
        match (crlf, lf) {
            (Some(c), Some(l)) if c <= l => LineEnding::Crlf,
            (Some(_), None) => LineEnding::Crlf,
            _ => LineEnding::Lf,
        }
    }

    /// Strip-then-convert; never `replace('\n', "\r\n")` directly, which
    /// doubles existing `\r\n` to `\r\r\n`.
    pub fn apply(self, text: &str) -> String {
        let lf = text.replace("\r\n", "\n").replace('\r', "\n");
        match self {
            LineEnding::Lf => lf,
            LineEnding::Crlf => lf.replace('\n', "\r\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_lf() {
        assert_eq!(LineEnding::detect("hello\nworld"), LineEnding::Lf);
    }

    #[test]
    fn detect_crlf() {
        assert_eq!(LineEnding::detect("hello\r\nworld"), LineEnding::Crlf);
    }

    #[test]
    fn detect_empty_defaults_to_lf() {
        assert_eq!(LineEnding::detect(""), LineEnding::Lf);
    }

    #[test]
    fn apply_lf_keeps_lf() {
        assert_eq!(LineEnding::Lf.apply("a\nb"), "a\nb");
    }

    #[test]
    fn apply_lf_strips_crlf_to_lf() {
        assert_eq!(LineEnding::Lf.apply("a\r\nb"), "a\nb");
    }

    #[test]
    fn apply_lf_strips_lone_cr() {
        assert_eq!(LineEnding::Lf.apply("a\rb"), "a\nb");
    }

    #[test]
    fn apply_crlf_from_lf() {
        assert_eq!(LineEnding::Crlf.apply("a\nb"), "a\r\nb");
    }

    #[test]
    fn apply_crlf_is_idempotent_on_crlf() {
        assert_eq!(LineEnding::Crlf.apply("a\r\nb"), "a\r\nb");
    }

    #[test]
    fn apply_crlf_does_not_double_existing_crlf() {
        // The bug we fixed: blind replace('\n', "\r\n") on text that already
        // had "\r\n" would have produced "a\r\r\nb".
        assert_eq!(LineEnding::Crlf.apply("a\r\nb"), "a\r\nb");
        assert_ne!(LineEnding::Crlf.apply("a\r\nb"), "a\r\r\nb");
    }

    #[test]
    fn apply_crlf_normalises_lone_cr() {
        assert_eq!(LineEnding::Crlf.apply("a\rb"), "a\r\nb");
    }

    // ===== edge cases =====

    #[test]
    fn detect_lone_cr_is_lf() {
        // Classic-Mac \r alone is treated as LF (we don't have a third variant).
        // Documented current behaviour; pin it so a regression is caught.
        assert_eq!(LineEnding::detect("a\rb"), LineEnding::Lf);
    }

    #[test]
    fn detect_mixed_crlf_first_wins() {
        // Mixed file with CRLF before LF → Crlf.
        assert_eq!(LineEnding::detect("a\r\nb\nc"), LineEnding::Crlf);
    }

    #[test]
    fn detect_mixed_lf_first_wins() {
        // Mixed file with LF before CRLF → Lf.
        assert_eq!(LineEnding::detect("a\nb\r\nc"), LineEnding::Lf);
    }

    #[test]
    fn detect_only_one_newline_crlf() {
        assert_eq!(LineEnding::detect("\r\n"), LineEnding::Crlf);
    }

    #[test]
    fn detect_only_one_newline_lf() {
        assert_eq!(LineEnding::detect("\n"), LineEnding::Lf);
    }

    #[test]
    fn apply_empty_string() {
        assert_eq!(LineEnding::Lf.apply(""), "");
        assert_eq!(LineEnding::Crlf.apply(""), "");
    }

    #[test]
    fn apply_no_newlines_at_all() {
        assert_eq!(LineEnding::Lf.apply("hello world"), "hello world");
        assert_eq!(LineEnding::Crlf.apply("hello world"), "hello world");
    }

    #[test]
    fn apply_crlf_on_mixed_input_is_uniform() {
        // Mixed CRLF + LF + lone CR → uniformly CRLF after apply.
        assert_eq!(
            LineEnding::Crlf.apply("a\r\nb\nc\rd"),
            "a\r\nb\r\nc\r\nd"
        );
    }

    #[test]
    fn apply_lf_on_mixed_input_is_uniform() {
        assert_eq!(LineEnding::Lf.apply("a\r\nb\nc\rd"), "a\nb\nc\nd");
    }

    #[test]
    fn apply_preserves_multibyte_chars() {
        // \u{4E2D} is the CJK "中" (3 bytes). Apply must not split it.
        let s = "a\u{4E2D}\r\nb";
        assert_eq!(LineEnding::Lf.apply(s), "a\u{4E2D}\nb");
        assert_eq!(LineEnding::Crlf.apply(s), s); // idempotent
    }

    #[test]
    fn apply_idempotent_when_pre_normalised() {
        // Applying twice == applying once.
        let inputs = [
            "",
            "no newlines",
            "a\nb",
            "a\r\nb",
            "a\rb",
            "mixed \r\n and \n and \r alone",
        ];
        for input in &inputs {
            for ending in &[LineEnding::Lf, LineEnding::Crlf] {
                let once = ending.apply(input);
                let twice = ending.apply(&once);
                assert_eq!(once, twice, "not idempotent: {ending:?} on {input:?}");
            }
        }
    }
}
