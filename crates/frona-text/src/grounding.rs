use std::ops::Range;

use unicode_categories::UnicodeCategories;

use crate::NormalizedString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroundingError {
    QuoteNotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundingMatch {
    pub normalized_range: Range<usize>,
    pub raw_range: Range<usize>,
    pub raw_span: String,
}

pub struct GroundingText {
    compact: NormalizedString,
    substantive_len: usize,
}

impl GroundingText {
    pub fn new(text: &str) -> Self {
        let mut compact = NormalizedString::from(text);
        compact.nfkc().case_fold().retain_chars(retained);
        let substantive_len = compact
            .get()
            .chars()
            .filter(|c| c.is_letter() || c.is_number())
            .count();
        Self {
            compact,
            substantive_len,
        }
    }

    pub fn comparison_stream(&self) -> &str {
        self.compact.get()
    }

    pub fn substantive_len(&self) -> usize {
        self.substantive_len
    }

    pub fn resolve(&self, quote: &str) -> Result<GroundingMatch, GroundingError> {
        let quote = Self::new(quote);
        if quote.substantive_len == 0 {
            return Err(GroundingError::QuoteNotFound);
        }
        self.resolve_compact(&quote)
    }

    /// Match a concrete structured value or alias. Unlike textual evidence quotes,
    /// legitimate short values are allowed; an empty compact value remains invalid.
    pub fn resolve_value(&self, value: &str) -> Result<GroundingMatch, GroundingError> {
        let value = Self::new(value);
        if value.substantive_len == 0 {
            return Err(GroundingError::QuoteNotFound);
        }
        self.resolve_compact(&value)
    }

    fn resolve_compact(&self, quote: &Self) -> Result<GroundingMatch, GroundingError> {
        let start = self
            .compact
            .get()
            .find(quote.compact.get())
            .ok_or(GroundingError::QuoteNotFound)?;
        let normalized_range = start..start + quote.compact.len();
        let raw_range = self
            .compact
            .splice_range_original(normalized_range.clone())
            .ok_or(GroundingError::QuoteNotFound)?;
        let raw_span = self.compact.get_original()[raw_range.clone()].to_string();
        Ok(GroundingMatch {
            normalized_range,
            raw_range,
            raw_span,
        })
    }
}

fn retained(c: char) -> bool {
    c.is_letter() || c.is_number() || c.is_mark()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multilingual_presentation_and_unicode_forms_resolve_to_raw_spans() {
        let cases = [
            (
                "**Tap   the main run button** — now",
                "tap the main run button",
                "Tap   the main run button",
            ),
            ("اضغط على الزِّر ١٢٣", "الزِّر ١٢٣", "الزِّر ١٢٣"),
            ("请按下运行按钮，然后等待。", "运行按钮", "运行按钮"),
            ("実行ボタンを押してください。", "実行ボタン", "実行ボタン"),
            (
                "İSTANBUL için çalıştır",
                "i\u{307}stanbul için",
                "İSTANBUL için",
            ),
            (
                "Die STRAẞE ist offen",
                "strasse ist offen",
                "STRAẞE ist offen",
            ),
            ("ΟΣ και ος είναι sigma", "οσ και οσ", "ΟΣ και ος"),
            ("अगला चरण चलाएँ", "अगला चरण", "अगला चरण"),
            (
                "Cafe\u{301} firmware",
                "Café firmware",
                "Cafe\u{301} firmware",
            ),
            (
                "Ｍｅｔａ ﬁrmware v1.71 /opt/FW.bin",
                "meta firmware v171 opt fw bin",
                "Ｍｅｔａ ﬁrmware v1.71 /opt/FW.bin",
            ),
        ];

        for (source, quote, expected_raw) in cases {
            let resolved = GroundingText::new(source).resolve(quote).unwrap();
            assert_eq!(
                resolved.raw_span, expected_raw,
                "source={source:?} quote={quote:?}"
            );
            assert_eq!(&source[resolved.raw_range], expected_raw);
        }
    }

    #[test]
    fn language_translation_and_meaningful_diacritic_changes_do_not_match() {
        assert_eq!(
            GroundingText::new("اضغط على زر التشغيل").resolve("Press the run button"),
            Err(GroundingError::QuoteNotFound),
        );
        assert_eq!(
            GroundingText::new("El próximo año").resolve("El proximo ano"),
            Err(GroundingError::QuoteNotFound),
        );
        assert_eq!(
            GroundingText::new("İstanbul").resolve("istanbul"),
            Err(GroundingError::QuoteNotFound),
        );
        assert_eq!(
            GroundingText::new("IĞDIR").resolve("ığdır"),
            Err(GroundingError::QuoteNotFound),
        );
    }

    #[test]
    fn exact_short_quotes_are_valid_evidence_but_empty_compact_quotes_are_rejected() {
        for quote in ["a", "AI", "42"] {
            assert!(
                GroundingText::new(&format!("before {quote} after"))
                    .resolve(quote)
                    .is_ok(),
                "quote={quote:?}",
            );
        }
        for quote in ["🔥", "---"] {
            assert_eq!(
                GroundingText::new(&format!("before {quote} after")).resolve(quote),
                Err(GroundingError::QuoteNotFound),
                "quote={quote:?}",
            );
        }
        assert!(GroundingText::new("value 5432").resolve("5432").is_ok());
        assert!(GroundingText::new("été").resolve("été").is_ok());
    }

    #[test]
    fn first_repeated_match_wins_and_boundaries_are_minimal() {
        let source = "**Press SET now**, then press SET later";
        let resolved = GroundingText::new(source).resolve("press set").unwrap();
        assert_eq!(resolved.raw_span, "Press SET");
        assert_eq!(resolved.raw_range, 2..11);
    }

    #[test]
    fn exposes_compact_stream_and_substantive_count() {
        let text = GroundingText::new("✅ **Straße ١٢**");
        assert_eq!(text.comparison_stream(), "strasse١٢");
        assert_eq!(text.substantive_len(), 9);
    }
}
