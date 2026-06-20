//! No read-before-edit guard: the uniqueness check self-heals if the file
//! drifted (no-match → agent re-reads → retries with corrected old_string).

use std::sync::Arc;

use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use crate::storage::service::StorageService;
use frona_derive::agent_tool;
use frona_text::{LineEnding, NormalizedString};

use super::super::sandbox::SandboxManager;
use super::super::{InferenceContext, ToolOutput};
use super::atomic_write;

const SNIPPET_CONTEXT_LINES: usize = 5;

pub struct EditTool {
    pub storage: StorageService,
    pub sandbox_manager: Arc<SandboxManager>,
    pub prompts: PromptLoader,
}

impl EditTool {
    pub fn new(
        storage: StorageService,
        sandbox_manager: Arc<SandboxManager>,
        prompts: PromptLoader,
    ) -> Self {
        Self { storage, sandbox_manager, prompts }
    }
}

fn normalize(ns: &mut NormalizedString) {
    ns.nfkc()
        .ascii_quotes()
        .ascii_dashes()
        .ascii_spaces()
        .collapse_whitespace_runs();
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditOutcome {
    Applied { rewritten: String, count: usize },
    NotFound,
    Ambiguous { count: usize },
    EmptyNeedle,
}

pub(crate) fn apply_edit(
    original: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> EditOutcome {
    let line_ending = LineEnding::detect(original);

    let mut file_ns = NormalizedString::from(original);
    normalize(&mut file_ns);

    let mut needle_ns = NormalizedString::from(old_string);
    normalize(&mut needle_ns);
    let needle = needle_ns.get();

    if needle.is_empty() {
        return EditOutcome::EmptyNeedle;
    }

    let haystack = file_ns.get();
    let mut matches: Vec<usize> = Vec::new();
    let mut search_from = 0usize;
    while let Some(idx) = haystack[search_from..].find(needle) {
        let abs = search_from + idx;
        matches.push(abs);
        search_from = abs + needle.len();
    }

    if matches.is_empty() {
        return EditOutcome::NotFound;
    }
    if matches.len() > 1 && !replace_all {
        return EditOutcome::Ambiguous { count: matches.len() };
    }

    let selected: Vec<usize> = if replace_all { matches.clone() } else { vec![matches[0]] };
    let needle_len = needle.len();

    // Normalise the replacement's line endings to match the file. The
    // bytes outside the match are spliced verbatim — they already use the
    // file's style — so applying restore() to the full rewritten buffer
    // (the previous design) corrupted existing `\r\n` into `\r\r\n`.
    let new_string_matched = line_ending.apply(new_string);

    let mut rewritten = String::with_capacity(original.len());
    let mut cursor = 0usize;
    for &nstart in &selected {
        let nend = nstart + needle_len;
        let Some(orig_range) = file_ns.splice_range_original(nstart..nend) else {
            return EditOutcome::NotFound;
        };
        if orig_range.start < cursor {
            continue;
        }
        rewritten.push_str(&original[cursor..orig_range.start]);
        rewritten.push_str(&new_string_matched);
        cursor = orig_range.end;
    }
    rewritten.push_str(&original[cursor..]);

    EditOutcome::Applied { rewritten, count: selected.len() }
}

#[agent_tool]
impl EditTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let path_arg = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'path' parameter".into()))?;
        let old_string = arguments
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'old_string' parameter".into()))?;
        let new_string = arguments
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'new_string' parameter".into()))?;
        let replace_all = arguments.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        let resolved = super::resolve_path(path_arg, &ctx.user.handle, &ctx.agent.handle, &self.storage)?;
        let sandbox = self.sandbox_manager.for_tool(ctx).await?;
        if !sandbox.is_writable(&resolved) {
            return Ok(ToolOutput::error(format!(
                "Edit denied by sandbox policy: {} (resolved: {})",
                path_arg,
                resolved.display(),
            )));
        }
        if !tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
            return Ok(ToolOutput::error(format!("file not found: {}", path_arg)));
        }

        let original = tokio::fs::read_to_string(&resolved).await.map_err(|e| {
            AppError::Internal(format!("read {}: {e}", resolved.display()))
        })?;

        match apply_edit(&original, old_string, new_string, replace_all) {
            EditOutcome::EmptyNeedle => {
                Ok(ToolOutput::error("old_string must not be empty".to_string()))
            }
            EditOutcome::NotFound => Ok(ToolOutput::error(format!(
                "old_string not found in {}. The file may have changed, or whitespace/quotes may not match.",
                path_arg
            ))),
            EditOutcome::Ambiguous { count } => Ok(ToolOutput::error(format!(
                "old_string matches {} locations in {}; provide more surrounding context to make it unique, or pass replace_all: true",
                count, path_arg,
            ))),
            EditOutcome::Applied { rewritten, count } => {
                atomic_write(&resolved, rewritten.as_bytes()).await?;
                let snippet = context_snippet(&original, &rewritten, SNIPPET_CONTEXT_LINES);
                let replacement_word = if count == 1 { "replacement" } else { "replacements" };
                let text = format!(
                    "Edit applied to {} ({} {}). Surrounding context:\n{}",
                    path_arg, count, replacement_word, snippet
                );
                Ok(ToolOutput::text(text))
            }
        }
    }
}

fn context_snippet(old: &str, new: &str, context: usize) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut first_change = new_lines.len();
    for (i, line) in new_lines.iter().enumerate() {
        if old_lines.get(i).copied() != Some(*line) {
            first_change = i;
            break;
        }
    }
    let start = first_change.saturating_sub(context);
    let end = (first_change + context + 1).min(new_lines.len());
    let width = (end + 1).to_string().len();
    new_lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>w$}\t{}", start + i + 1, line, w = width))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(file: &str, old: &str, new: &str) -> EditOutcome {
        apply_edit(file, old, new, false)
    }

    #[test]
    fn exact_byte_match() {
        let r = apply("let s = \"hello\";", "\"hello\"", "\"world\"");
        match r {
            EditOutcome::Applied { rewritten, count } => {
                assert_eq!(rewritten, "let s = \"world\";");
                assert_eq!(count, 1);
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn smart_quotes_in_needle_match_ascii_in_file() {
        // old_string has curly quotes; file has ASCII quotes. Should match.
        let r = apply("let s = \"hello\";", "\u{201C}hello\u{201D}", "world");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert_eq!(rewritten, "let s = world;");
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn ascii_quotes_in_needle_match_smart_in_file() {
        let r = apply(
            "let s = \u{201C}hello\u{201D};",
            "\"hello\"",
            "\"world\"",
        );
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                // The file's smart quotes were absorbed by the match; the
                // splice replaces the whole quoted region with the new ASCII
                // form. Bytes outside the match are untouched.
                assert!(rewritten.starts_with("let s = "));
                assert!(rewritten.ends_with(";"));
                assert!(rewritten.contains("\"world\""));
                // The original smart-quoted region is gone.
                assert!(!rewritten.contains('\u{201C}'));
                assert!(!rewritten.contains('\u{201D}'));
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn em_dash_in_needle_matches_ascii_hyphen_in_file() {
        let r = apply("a - b", "a \u{2014} b", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "X"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn tabs_in_needle_match_spaces_in_file() {
        // File has multiple spaces; needle has a tab. Whitespace runs collapse
        // to a single space on both sides, so the match succeeds.
        let r = apply("a    b", "a\tb", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "X"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn spaces_in_needle_match_tabs_in_file() {
        let r = apply("a\tb", "a b", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "X"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn crlf_file_with_lf_needle_preserves_crlf() {
        // LLM emits LF needle; file is CRLF. The match should fire, and the
        // rewritten file must keep CRLF outside the edit. No \r\r\n.
        let file = "a\r\nb\r\nc\r\n";
        let r = apply(file, "a\nb", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert_eq!(rewritten, "X\r\nc\r\n");
                assert!(!rewritten.contains("\r\r\n"), "no doubled CR: {rewritten:?}");
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn crlf_file_with_multiline_lf_new_string_gets_crlf_inserted() {
        // LLM emits a multi-line new_string with LF only. Splicing into a
        // CRLF file should convert the new_string's LFs to CRLF so the
        // resulting file is uniformly CRLF.
        let file = "before\r\nmid\r\nafter\r\n";
        let r = apply(file, "mid", "line1\nline2");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert_eq!(rewritten, "before\r\nline1\r\nline2\r\nafter\r\n");
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn lf_file_with_crlf_new_string_gets_lf_converted() {
        // Inverse: LLM emits CRLF new_string; file is LF. Splice should
        // convert new_string to LF.
        let file = "a\nb\nc\n";
        let r = apply(file, "b", "X\r\nY");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert_eq!(rewritten, "a\nX\nY\nc\n");
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn trailing_whitespace_drift_still_matches() {
        // File has trailing spaces; needle doesn't. The whitespace-run
        // collapse + ASCII space fold makes them match.
        let r = apply("hello   \nworld", "hello\nworld", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "X"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn no_match_returns_not_found() {
        let r = apply("hello world", "missing", "replacement");
        assert_eq!(r, EditOutcome::NotFound);
    }

    #[test]
    fn multiple_matches_without_replace_all_is_ambiguous() {
        let r = apply("foo foo foo", "foo", "bar");
        match r {
            EditOutcome::Ambiguous { count } => assert_eq!(count, 3),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn replace_all_handles_normalised_duplicates() {
        // The file has both ASCII "foo" and curly "foo" — after normalisation
        // both look identical. replace_all should replace both.
        let file = "\"foo\" and \u{201C}foo\u{201D}";
        let r = apply_edit(file, "\"foo\"", "X", true);
        match r {
            EditOutcome::Applied { rewritten, count } => {
                assert_eq!(count, 2);
                assert_eq!(rewritten, "X and X");
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn empty_needle_rejected() {
        let r = apply("foo", "", "bar");
        assert_eq!(r, EditOutcome::EmptyNeedle);
    }

    #[test]
    fn bytes_outside_edit_are_byte_identical() {
        // File has a smart quote OUTSIDE the edited region. After Edit, that
        // smart quote must still be present in the rewritten file — Edit must
        // not silently rewrite untouched regions.
        let file = "before \u{201C}untouched\u{201D} middle \"target\" after";
        let r = apply(file, "\"target\"", "[replaced]");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert!(rewritten.contains('\u{201C}'), "smart left quote lost: {rewritten:?}");
                assert!(rewritten.contains('\u{201D}'), "smart right quote lost: {rewritten:?}");
                assert!(rewritten.contains("[replaced]"));
                assert!(!rewritten.contains("\"target\""));
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    // ===== adversarial file content =====
    // The fundamental Edit guarantee: bytes outside the matched region are
    // byte-identical to the original. These tests stress the most likely
    // ways that property could break.

    #[test]
    fn match_at_exact_start_of_file() {
        let r = apply("FOO bar baz", "FOO", "QUX");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "QUX bar baz"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn match_at_exact_end_of_file() {
        let r = apply("foo bar BAZ", "BAZ", "QUX");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "foo bar QUX"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn match_spans_whole_file() {
        let r = apply("entire", "entire", "replaced");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "replaced"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn empty_file_no_match() {
        let r = apply("", "x", "y");
        assert_eq!(r, EditOutcome::NotFound);
    }

    #[test]
    fn single_byte_file_match() {
        let r = apply("x", "x", "y");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "y"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn single_byte_file_no_match() {
        let r = apply("x", "y", "z");
        assert_eq!(r, EditOutcome::NotFound);
    }

    #[test]
    fn file_without_trailing_newline_preserved() {
        // No trailing \n. After edit, still no trailing \n.
        let r = apply("hello", "hello", "world");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert_eq!(rewritten, "world");
                assert!(!rewritten.ends_with('\n'));
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn file_with_trailing_newline_preserved() {
        let r = apply("hello\n", "hello", "world");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "world\n"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn empty_new_string_acts_as_deletion() {
        // Empty new_string is allowed (only old_string can't be empty).
        let r = apply("before TARGET after", "TARGET", "");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "before  after"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn old_string_substring_of_new_string_no_recursion() {
        // Critical: replacing "foo" with "foofoo" must NOT re-match in the
        // replacement and loop forever.
        let r = apply("foo bar", "foo", "foofoo");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "foofoo bar"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn old_string_contains_new_string_no_recursion() {
        // The other direction: old contains new. Should be plain replacement.
        let r = apply("foofoo bar", "foofoo", "foo");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "foo bar"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn replace_all_with_three_matches() {
        let r = apply_edit("a X b X c X d", "X", "Y", true);
        match r {
            EditOutcome::Applied { rewritten, count } => {
                assert_eq!(rewritten, "a Y b Y c Y d");
                assert_eq!(count, 3);
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn replace_all_adjacent_matches() {
        // No characters between matches.
        let r = apply_edit("XXX", "X", "Y", true);
        match r {
            EditOutcome::Applied { rewritten, count } => {
                assert_eq!(rewritten, "YYY");
                assert_eq!(count, 3);
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn multibyte_chars_outside_edit_byte_identical() {
        // The file contains a CJK char OUTSIDE the edit region. After Edit,
        // that char must still be exactly 3 bytes in the same position.
        let file = "\u{4E2D}\u{4E2D} target \u{4E2D}";
        let r = apply(file, "target", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert_eq!(rewritten, "\u{4E2D}\u{4E2D} X \u{4E2D}");
                // Sanity: the multibyte chars are intact and at the right
                // byte positions.
                assert!(rewritten.is_char_boundary(0));
                assert!(rewritten.is_char_boundary(3)); // after first 中
                assert!(rewritten.is_char_boundary(6)); // after second 中
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn multibyte_chars_inside_edit_replaced() {
        let r = apply("a \u{4E2D}\u{4E2D} b", "\u{4E2D}\u{4E2D}", "REPLACED");
        match r {
            EditOutcome::Applied { rewritten, .. } => assert_eq!(rewritten, "a REPLACED b"),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn bom_outside_edit_preserved() {
        // BOM (U+FEFF) at the start of the file must survive the edit.
        let file = "\u{FEFF}let s = TARGET;";
        let r = apply(file, "TARGET", "VALUE");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                assert!(rewritten.starts_with('\u{FEFF}'), "BOM lost: {rewritten:?}");
                assert!(rewritten.contains("VALUE"));
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn combining_accents_outside_edit_preserved() {
        // Decomposed accents outside the edit region must survive unchanged.
        // NFKC composes them in the *normalised* form for matching, but the
        // splice into original bytes preserves the decomposed form.
        let file = "e\u{0301} target";
        let r = apply(file, "target", "X");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                // The decomposed form must still be there.
                assert!(
                    rewritten.contains("e\u{0301}"),
                    "decomposed accent lost: {rewritten:?}",
                );
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_outside_edit_is_byte_identical() {
        // File has weird whitespace runs OUTSIDE the edited region. They must
        // survive the edit unchanged.
        let file = "a    b\n\n\nc = 1\nd   e";
        let r = apply(file, "c = 1", "c = 2");
        match r {
            EditOutcome::Applied { rewritten, .. } => {
                // The 4-space run before 'b' should survive verbatim.
                assert!(rewritten.contains("a    b"), "left whitespace lost: {rewritten:?}");
                // The 3-newline gap should survive.
                assert!(rewritten.contains("b\n\n\nc"), "newline gap lost: {rewritten:?}");
                // The 3-space run between d and e should survive.
                assert!(rewritten.contains("d   e"), "right whitespace lost: {rewritten:?}");
                // The edit itself applied.
                assert!(rewritten.contains("c = 2"));
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }
}
