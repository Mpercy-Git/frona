//! Message splitter for channel adapters.
//!
//! Each adapter holds one splitter constructed with its per-provider limits
//! (Telegram 4096, Discord 2000, SMS 1600, etc.) and calls it before sending
//! to chunk long agent replies into N sequential messages.
//!
//! Four concrete splitter types, no trait:
//! - [`PlainSplitter`] — Slack, SMS. Plain-text input.
//! - [`MarkdownSplitter`] — Discord, WhatsApp. Fence-aware GFM markdown.
//! - [`TelegramMarkdownV2Splitter`] — Telegram. Markdown rules plus
//!   never-split-between-`\`-and-escaped-char.
//! - [`SignalSplitter`] — Signal. Composes [`MarkdownSplitter`] then runs
//!   [`super::markdown::to_signal`] per chunk so each chunk gets its own
//!   balanced `SignalText { body, ranges }`.

use tokio_util::sync::CancellationToken;

use super::super::error::ChannelError;
use super::super::models::ChannelCtx;
use crate::credential::share::service::ShareService;

/// Per-message values supplied at call time (chat_id, user_id are needed
/// only when `hard_limit` fires and a chat-share URL must be minted).
#[derive(Debug, Clone, Copy)]
pub(super) struct SplitCtx<'a> {
    pub chat_id: &'a str,
    pub user_id: &'a str,
}

/// Fence state at a given position. Used by [`MarkdownSplitter`] and
/// [`TelegramMarkdownV2Splitter`] to avoid splitting inside code regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct FenceState {
    /// `Some` if currently inside a block fence (``` or ~~~). Carries the
    /// marker char and the opening run length so we know how many of the
    /// matching marker are required to close.
    pub block: Option<BlockFence>,
    /// `Some(n)` if currently inside an N-backtick inline span. Reset at
    /// every line break.
    pub inline_open: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockFence {
    pub marker: u8,
    pub open_len: usize,
}

impl FenceState {
    #[cfg(test)]
    pub fn is_safe(&self) -> bool {
        self.block.is_none() && self.inline_open.is_none()
    }
}

/// Find the best byte offset to cut `text` such that the chunk fits within
/// `target` bytes. Preference order: paragraph (`\n\n`) > newline (`\n`) >
/// space (` `) > UTF-8 char boundary. Returns `text.len()` if `text` is
/// already within `target`.
pub(super) fn find_boundary(text: &str, target: usize) -> usize {
    if text.len() <= target {
        return text.len();
    }

    // Walk back from `target` to the nearest char boundary so substring
    // operations don't panic.
    let mut upper = target;
    while upper > 0 && !text.is_char_boundary(upper) {
        upper -= 1;
    }
    if upper == 0 {
        return 0;
    }
    let slice = &text[..upper];

    if let Some(i) = slice.rfind("\n\n") {
        return i + 2;
    }
    if let Some(i) = slice.rfind('\n') {
        return i + 1;
    }
    if let Some(i) = slice.rfind(' ') {
        return i + 1;
    }
    upper
}

/// Compute the fence state at `cursor` (a byte offset into `text`). Scans
/// from the start of `text` because fence state is path-dependent.
///
/// - Block fences (``` / ~~~) open at line start with N≥3 markers and close
///   with M≥N matching markers on their own line. Inside a block fence,
///   inline backtick scanning is suppressed (content is literal).
/// - Inline backtick spans (single or multi-backtick) open with a run of N
///   backticks and close with another run of exactly N. Reset at each line
///   break — per CommonMark inline spans usually don't cross lines, and
///   forcing this keeps the scanner cheap.
#[cfg(test)]
pub(super) fn fence_depth_at(text: &str, cursor: usize) -> FenceState {
    fence_depth_from(text, cursor, FenceState::default())
}

/// Like [`fence_depth_at`] but seeded with `entry` — the fence state at
/// position 0 of `text`. Used by [`MarkdownSplitter`] to continue scanning
/// across chunk boundaries when a previous chunk ended inside an open block
/// fence (close+reopen pattern).
pub(super) fn fence_depth_from(text: &str, cursor: usize, entry: FenceState) -> FenceState {
    let mut state = entry;
    let bytes = text.as_bytes();
    let end = cursor.min(bytes.len());
    let mut line_start = 0;

    while line_start < end {
        let line_end = bytes[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| line_start + i)
            .unwrap_or(bytes.len());
        let line = &text[line_start..line_end];

        if let Some(bf) = state.block {
            if is_closing_fence(line, bf) {
                state.block = None;
            }
        } else if let Some(bf) = parse_opening_fence(line) {
            state.block = Some(bf);
            state.inline_open = None;
        } else {
            let scan_end = end.min(line_end);
            scan_inline_backticks(&text[line_start..scan_end], &mut state.inline_open);
        }

        // Cursor on this line — return without resetting inline_open.
        if line_end >= end {
            return state;
        }

        // Inline spans never cross newlines.
        line_start = line_end + 1;
        if state.block.is_none() {
            state.inline_open = None;
        }
    }
    state
}

fn parse_opening_fence(line: &str) -> Option<BlockFence> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let marker = *bytes.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let mut n = 0;
    while n < bytes.len() && bytes[n] == marker {
        n += 1;
    }
    if n < 3 {
        return None;
    }
    // CommonMark: an info string after a backtick fence cannot itself
    // contain backticks (otherwise it's ambiguous with inline code).
    if marker == b'`' && bytes[n..].contains(&b'`') {
        return None;
    }
    Some(BlockFence { marker, open_len: n })
}

fn is_closing_fence(line: &str, bf: BlockFence) -> bool {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return false;
    }
    let bytes = trimmed.as_bytes();
    let mut n = 0;
    while n < bytes.len() && bytes[n] == bf.marker {
        n += 1;
    }
    if n < bf.open_len {
        return false;
    }
    // After the run, only whitespace (CommonMark §4.5).
    bytes[n..].iter().all(|&b| b == b' ' || b == b'\t')
}

fn scan_inline_backticks(chunk: &str, open: &mut Option<usize>) {
    let bytes = chunk.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let mut n = 0;
        while i + n < bytes.len() && bytes[i + n] == b'`' {
            n += 1;
        }
        match *open {
            None => *open = Some(n),
            Some(m) if m == n => *open = None,
            // Mismatched run inside an open span — treated as content.
            Some(_) => {}
        }
        i += n;
    }
}

const OVERFLOW_PREFIX: &str = "\n\nFull reply: ";

/// Truncate `body` and append `\n\nFull reply: {url}` so the total fits
/// within `total_limit`. `body` is cut at the best boundary (paragraph >
/// line > word > char) inside the body-budget and trailing whitespace is
/// trimmed so the suffix sits cleanly.
pub(super) fn append_overflow(body: &str, url: &str, total_limit: usize) -> String {
    let suffix_len = OVERFLOW_PREFIX.len() + url.len();
    let body_budget = total_limit.saturating_sub(suffix_len);
    let cut = find_boundary(body, body_budget);
    let head = body[..cut].trim_end();
    format!("{head}{OVERFLOW_PREFIX}{url}")
}

/// Look up the cached chat-share URL for `(user_id, chat_id)` or mint a new
/// one lazily. Returns the full `{base_url}/s/{id}` URL.
///
/// Races against `cancel` so a stop_channel signal doesn't leave the send
/// loop blocked on a slow share-service call.
pub(super) async fn ensure_chat_share(
    share: &ShareService,
    base_url: &str,
    ttl_secs: u64,
    cancel: &CancellationToken,
    chat_id: &str,
    user_id: &str,
) -> Result<String, ChannelError> {
    let lookup = share.lookup_or_issue_chat(chat_id, user_id, ttl_secs);
    tokio::select! {
        result = lookup => match result {
            Ok(id) => Ok(format!("{base_url}/s/{id}")),
            Err(e) => Err(ChannelError::transient(format!("issue chat share: {e}"))),
        },
        _ = cancel.cancelled() => Err(ChannelError::transient("channel cancelled")),
    }
}

/// Plain-text splitter for adapters whose wire format has no markdown
/// (Slack `to_plain`, SMS `to_plain`). When `hard_limit` is `Some(N)` and
/// the input exceeds it, the splitter returns a single chunk truncated to
/// fit `provider_limit - "\n\nFull reply: {url}".len()` plus the overflow
/// URL (lookup-or-mint of a `ShareKind::Chat` short link).
pub(super) struct PlainSplitter {
    provider_limit: usize,
    hard_limit: Option<usize>,
}

impl PlainSplitter {
    pub fn new(provider_limit: usize, hard_limit: Option<usize>) -> Self {
        Self { provider_limit, hard_limit }
    }

    pub async fn split(
        &self,
        text: &str,
        ctx: &ChannelCtx,
        sctx: SplitCtx<'_>,
    ) -> Result<Vec<String>, ChannelError> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        if let Some(hard) = self.hard_limit
            && text.len() > hard
        {
            let url = ensure_chat_share(
                &ctx.share_service,
                &ctx.base_url,
                ctx.share_ttl_secs,
                &ctx.cancel,
                sctx.chat_id,
                sctx.user_id,
            )
            .await?;
            return Ok(vec![append_overflow(text, &url, self.provider_limit)]);
        }

        Ok(silent_split_plain(text, self.provider_limit))
    }
}

/// Silent multi-chunk split for plain text — no share/I/O, no fence
/// awareness. Pure function reusable by adapter fallback paths that need a
/// plain-text chunker (e.g., Telegram's MarkdownV2-rejection fallback) and
/// by [`PlainSplitter`]'s silent-mode branch.
pub(super) fn silent_split_plain(text: &str, provider_limit: usize) -> Vec<String> {
    if text.len() <= provider_limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= provider_limit {
            chunks.push(remaining.to_string());
            break;
        }
        let cut = find_boundary(remaining, provider_limit);
        if cut == 0 {
            // Pathological input (e.g. tiny limit + multibyte char). Force
            // forward to the next char boundary so we always make progress.
            let mut i = 1;
            while i < remaining.len() && !remaining.is_char_boundary(i) {
                i += 1;
            }
            chunks.push(remaining[..i].to_string());
            remaining = &remaining[i..];
        } else {
            chunks.push(remaining[..cut].to_string());
            remaining = &remaining[cut..];
        }
    }
    chunks
}

/// GFM-markdown splitter for adapters whose wire format is markdown
/// (Discord raw, WhatsApp Cloud/User via `to_whatsapp`, also composed by
/// [`SignalSplitter`]). Fence-aware: never splits inside an open block
/// fence (close+reopen across the boundary) or inside an inline backtick
/// span (backtrack to outside the span).
pub(super) struct MarkdownSplitter {
    provider_limit: usize,
    #[allow(dead_code)] // hard_limit reserved for future truncate-with-link adapters
    hard_limit: Option<usize>,
}

impl MarkdownSplitter {
    pub fn new(provider_limit: usize, hard_limit: Option<usize>) -> Self {
        Self { provider_limit, hard_limit }
    }

    pub fn split(&self, text: &str) -> Vec<String> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        if text.len() <= self.provider_limit {
            return vec![text.to_string()];
        }
        split_markdown_aware(text, self.provider_limit)
    }
}

/// Markdown-aware split. Tracks fence state across chunk boundaries so a
/// chunk that ends inside an open block fence gets a `\n```\n` close
/// appended, and the next chunk gets the matching opener prepended.
fn split_markdown_aware(text: &str, provider_limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cursor = 0;
    let mut entry = FenceState::default();
    let total = text.len();

    while cursor < total {
        let remaining = &text[cursor..];

        // If everything left fits, emit and we're done. If we entered with
        // an open block fence, the previous chunk already added the close
        // marker and we add the opener here.
        // Reopen the fence at the head of each chunk that inherits an open
        // block fence from its predecessor — the previous chunk emitted the
        // matching close.
        let prefix = open_fence_marker(entry.block);
        if remaining.len() + prefix.len() <= provider_limit {
            chunks.push(format!("{prefix}{remaining}"));
            break;
        }

        let budget = provider_limit.saturating_sub(prefix.len());
        let candidate = find_boundary(remaining, budget);
        let state_at_candidate = fence_depth_from(remaining, candidate, entry);

        let (cut, chunk_close) = match (state_at_candidate.block, state_at_candidate.inline_open) {
            (None, None) => (candidate, String::new()),
            (block, Some(_)) => {
                if let Some(safe) = backtrack_to_safe(remaining, candidate, entry) {
                    let state_at_safe = fence_depth_from(remaining, safe, entry);
                    let close = close_fence_marker(state_at_safe.block);
                    (safe, close)
                } else {
                    // No safe inline boundary — split inside the span.
                    let close = close_fence_marker(block);
                    (candidate, close)
                }
            }
            (Some(bf), None) => (candidate, close_fence_marker(Some(bf))),
        };

        if cut == 0 {
            // Pathological input. Force one char so the loop terminates.
            let mut i = 1;
            while i < remaining.len() && !remaining.is_char_boundary(i) {
                i += 1;
            }
            chunks.push(format!("{prefix}{}", &remaining[..i]));
            cursor += i;
            entry = fence_depth_from(text, cursor, FenceState::default());
            continue;
        }

        let body = &remaining[..cut];
        chunks.push(format!("{prefix}{body}{chunk_close}"));

        cursor += cut;
        entry = fence_depth_from(text, cursor, FenceState::default());
    }

    chunks
}

/// `"```\n"` / `"~~~\n"` to reopen an unclosed fence at the start of the
/// next chunk. Empty if no fence is open.
fn open_fence_marker(block: Option<BlockFence>) -> String {
    match block {
        None => String::new(),
        Some(bf) => {
            let marker = std::iter::repeat_n(bf.marker as char, bf.open_len)
                .collect::<String>();
            format!("{marker}\n")
        }
    }
}

/// `"\n```"` / `"\n~~~"` to close an open fence at the end of the current
/// chunk. Empty if no fence is open.
fn close_fence_marker(block: Option<BlockFence>) -> String {
    match block {
        None => String::new(),
        Some(bf) => {
            let marker = std::iter::repeat_n(bf.marker as char, bf.open_len)
                .collect::<String>();
            format!("\n{marker}")
        }
    }
}

/// Telegram MarkdownV2 splitter — markdown rules plus the rule that a cut
/// must never land between `\` and the character it's escaping. The
/// escape-aware fence model is otherwise identical to [`MarkdownSplitter`].
pub(super) struct TelegramMarkdownV2Splitter {
    provider_limit: usize,
    #[allow(dead_code)]
    hard_limit: Option<usize>,
}

impl TelegramMarkdownV2Splitter {
    pub fn new(provider_limit: usize, hard_limit: Option<usize>) -> Self {
        Self { provider_limit, hard_limit }
    }

    pub fn split(&self, text: &str) -> Vec<String> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        if text.len() <= self.provider_limit {
            return vec![text.to_string()];
        }
        split_markdown_v2_aware(text, self.provider_limit)
    }
}

/// Same algorithm as [`split_markdown_aware`] but with escape-pair safety:
/// if a candidate cut lands such that `text[..cut]` ends with an unmatched
/// `\`, advance the cut so the escape pair stays together.
fn split_markdown_v2_aware(text: &str, provider_limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cursor = 0;
    let mut entry = FenceState::default();
    let total = text.len();

    while cursor < total {
        let remaining = &text[cursor..];

        let prefix = open_fence_marker(entry.block);
        if remaining.len() + prefix.len() <= provider_limit {
            chunks.push(format!("{prefix}{remaining}"));
            break;
        }

        let budget = provider_limit.saturating_sub(prefix.len());
        let raw_candidate = find_boundary(remaining, budget);
        let candidate = avoid_split_after_backslash(remaining, raw_candidate);
        let state_at_candidate = fence_depth_from(remaining, candidate, entry);

        let (cut, chunk_close) = match (state_at_candidate.block, state_at_candidate.inline_open) {
            (None, None) => (candidate, String::new()),
            (block, Some(_)) => {
                if let Some(safe) = backtrack_to_safe(remaining, candidate, entry) {
                    let safe = avoid_split_after_backslash(remaining, safe);
                    let state_at_safe = fence_depth_from(remaining, safe, entry);
                    let close = close_fence_marker(state_at_safe.block);
                    (safe, close)
                } else {
                    let close = close_fence_marker(block);
                    (candidate, close)
                }
            }
            (Some(bf), None) => (candidate, close_fence_marker(Some(bf))),
        };

        if cut == 0 {
            let mut i = 1;
            while i < remaining.len() && !remaining.is_char_boundary(i) {
                i += 1;
            }
            chunks.push(format!("{prefix}{}", &remaining[..i]));
            cursor += i;
            entry = fence_depth_from(text, cursor, FenceState::default());
            continue;
        }

        let body = &remaining[..cut];
        chunks.push(format!("{prefix}{body}{chunk_close}"));

        cursor += cut;
        entry = fence_depth_from(text, cursor, FenceState::default());
    }

    chunks
}

/// If `cut` lands such that `text[..cut]` ends with an odd run of `\`,
/// advance `cut` by one char so the escaped char stays in the same chunk.
fn avoid_split_after_backslash(text: &str, cut: usize) -> usize {
    if cut == 0 || cut >= text.len() {
        return cut;
    }
    let bytes = text.as_bytes();
    let mut backslashes = 0;
    let mut i = cut;
    while i > 0 && bytes[i - 1] == b'\\' {
        backslashes += 1;
        i -= 1;
    }
    if backslashes % 2 == 1 {
        // Odd → the last `\` is opening an escape; consume the escaped char.
        let mut new_cut = cut + 1;
        while new_cut < text.len() && !text.is_char_boundary(new_cut) {
            new_cut += 1;
        }
        return new_cut;
    }
    cut
}

/// Signal splitter. Composes [`MarkdownSplitter`] over raw markdown input,
/// then runs [`super::markdown::to_signal`] per chunk so each chunk gets
/// its own `SignalText { body, ranges }` with ranges scoped to that chunk.
/// No range arithmetic — formatting that fits inside a chunk is preserved;
/// formatting that crosses a chunk boundary degrades to plain text in both
/// chunks (paragraph-biased cuts make this rare).
pub(super) struct SignalSplitter {
    inner: MarkdownSplitter,
}

impl SignalSplitter {
    pub fn new(inner: MarkdownSplitter) -> Self {
        Self { inner }
    }

    pub fn split(&self, raw_markdown: &str) -> Vec<super::markdown::SignalText> {
        self.inner
            .split(raw_markdown)
            .into_iter()
            .map(|chunk| super::markdown::to_signal(&chunk))
            .collect()
    }
}

/// Walk back from `from` to find the last position where neither a block
/// fence nor an inline span is open. Returns `None` if the entire prefix
/// is inside an inline span (rare; degenerate input).
fn backtrack_to_safe(text: &str, from: usize, entry: FenceState) -> Option<usize> {
    let mut pos = from;
    while pos > 0 {
        pos -= 1;
        if !text.is_char_boundary(pos) {
            continue;
        }
        let state = fence_depth_from(text, pos, entry);
        if state.inline_open.is_none() {
            return Some(pos);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_boundary_empty_returns_zero() {
        assert_eq!(find_boundary("", 100), 0);
    }

    #[test]
    fn find_boundary_under_target_returns_len() {
        assert_eq!(find_boundary("hello", 100), 5);
    }

    #[test]
    fn find_boundary_prefers_paragraph_break() {
        let text = "para1\n\npara2 more text";
        assert_eq!(find_boundary(text, 12), 7);
        assert_eq!(&text[..7], "para1\n\n");
        assert_eq!(&text[7..], "para2 more text");
    }

    #[test]
    fn find_boundary_falls_back_to_newline() {
        assert_eq!(find_boundary("line one\nline two longer", 15), 9);
    }

    #[test]
    fn find_boundary_falls_back_to_space() {
        assert_eq!(find_boundary("abc def ghi", 8), 8);
    }

    #[test]
    fn find_boundary_falls_back_to_char_boundary_on_single_long_word() {
        assert_eq!(find_boundary("abcdefghijklmnop", 8), 8);
    }

    #[test]
    fn find_boundary_respects_multibyte_char_boundary() {
        let text = "héllo wörld extra";
        let cut = find_boundary(text, 8);
        assert!(text.is_char_boundary(cut));
        assert!(cut <= 8);
    }

    #[test]
    fn fence_depth_no_fences_is_safe() {
        let text = "just plain prose, no markers";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());
    }

    #[test]
    fn fence_depth_inside_block_fence() {
        let text = "intro\n```\ncode here";
        let state = fence_depth_at(text, text.len());
        assert!(state.block.is_some(), "should be inside an open block fence");
        assert_eq!(state.block.unwrap().marker, b'`');
        assert_eq!(state.block.unwrap().open_len, 3);
    }

    #[test]
    fn fence_depth_after_close_returns_to_safe() {
        let text = "intro\n```\ncode\n```\nafter";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe(), "after closing fence should be safe");
    }

    #[test]
    fn fence_depth_nested_three_backtick_inside_four_backtick_is_content() {
        let text = "````\n```\nstill inside\n````\nafter";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());
    }

    #[test]
    fn fence_depth_variable_length_requires_matching_close() {
        let text = "````\n```\n````\nafter";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());
    }

    #[test]
    fn fence_depth_tilde_tracked_independently_from_backtick() {
        let text = "~~~\ncode\n~~~\nafter";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());

        let mixed = "~~~\n```\nstill inside\n~~~\nafter";
        let state2 = fence_depth_at(mixed, mixed.len());
        assert!(state2.is_safe());
    }

    #[test]
    fn fence_depth_inline_backtick_open_and_close() {
        let text = "before `code` after";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());
    }

    #[test]
    fn fence_depth_inside_inline_backtick_span() {
        let text = "before `code midway";
        let state = fence_depth_at(text, text.len());
        assert_eq!(state.inline_open, Some(1));
    }

    #[test]
    fn fence_depth_multi_backtick_inline_matches_by_length() {
        let text = "before ``has ` inside`` after";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());
    }

    #[test]
    fn fence_depth_inline_reset_at_line_break() {
        let text = "broken `unclosed\nnext line";
        let state = fence_depth_at(text, text.len());
        assert!(state.is_safe());
    }

    #[test]
    fn append_overflow_truncates_body_and_appends_link() {
        let body = "a".repeat(1000);
        let url = "https://app.host/s/8Dbcv_bu";
        let out = append_overflow(&body, url, 100);
        assert!(out.len() <= 100, "must fit total_limit: {}", out.len());
        assert!(out.ends_with(url), "ends with the overflow URL");
        assert!(out.contains("\n\nFull reply: "), "carries the suffix prefix");
    }

    #[test]
    fn append_overflow_prefers_paragraph_boundary_inside_budget() {
        let body = "first paragraph content goes here\n\nsecond paragraph stuff";
        let url = "https://x/s/y";
        let total_limit = body.find("\n\n").unwrap() + 2 + OVERFLOW_PREFIX.len() + url.len();
        let out = append_overflow(body, url, total_limit);
        assert_eq!(
            out,
            format!("first paragraph content goes here{OVERFLOW_PREFIX}{url}")
        );
    }

    #[test]
    fn append_overflow_zero_body_budget_emits_just_the_link() {
        let url = "https://x/s/y";
        let total = OVERFLOW_PREFIX.len() + url.len();
        let out = append_overflow("anything at all", url, total);
        assert_eq!(out, format!("{OVERFLOW_PREFIX}{url}"));
    }

    #[test]
    fn silent_split_under_limit_returns_one_chunk() {
        assert_eq!(silent_split_plain("hello", 100), vec!["hello".to_string()]);
    }

    #[test]
    fn silent_split_breaks_on_paragraph_boundary() {
        let text = "para one\n\npara two\n\npara three";
        let chunks = silent_split_plain(text, 12);
        for (i, c) in chunks.iter().enumerate() {
            assert!(
                c.len() <= 12 || i == chunks.len() - 1,
                "chunk {i} too large: {c:?}"
            );
        }
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn silent_split_multibyte_safe() {
        let text = "éàü ".repeat(100);
        let chunks = silent_split_plain(&text, 30);
        assert_eq!(chunks.join(""), text);
    }

    async fn test_share_service() -> ShareService {
        use std::sync::Arc;
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        use crate::credential::share::models::Share;
        use crate::credential::share::repository::ShareRepository;
        use crate::db::repo::generic::SurrealRepo;

        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        let repo: Arc<dyn ShareRepository> = Arc::new(SurrealRepo::<Share>::new(db));
        ShareService::new(repo, 3600)
    }

    #[tokio::test]
    async fn ensure_chat_share_returns_full_url() {
        let svc = test_share_service().await;
        let cancel = CancellationToken::new();
        let url = ensure_chat_share(&svc, "https://app.host", 3600, &cancel, "chat-1", "user-1")
            .await
            .unwrap();
        assert!(url.starts_with("https://app.host/s/"));
        assert_eq!(url.len(), "https://app.host/s/".len() + 8);
    }

    #[tokio::test]
    async fn ensure_chat_share_reuses_on_second_call() {
        let svc = test_share_service().await;
        let cancel = CancellationToken::new();
        let first = ensure_chat_share(&svc, "https://app.host", 3600, &cancel, "chat-1", "user-1")
            .await
            .unwrap();
        let second = ensure_chat_share(&svc, "https://app.host", 3600, &cancel, "chat-1", "user-1")
            .await
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn markdown_under_limit_returns_one_chunk() {
        let splitter = MarkdownSplitter::new(2000, None);
        assert_eq!(splitter.split("hello world"), vec!["hello world".to_string()]);
    }

    #[test]
    fn markdown_splits_on_newline_boundary() {
        let splitter = MarkdownSplitter::new(2000, None);
        let line = "a".repeat(500);
        let blob = format!("{line}\n{line}\n{line}\n{line}\n{line}");
        let chunks = splitter.split(&blob);
        assert!(chunks.len() >= 2, "expected ≥2 chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(c.len() <= 2000, "chunk exceeds limit: {}", c.len());
        }
    }

    #[test]
    fn markdown_empty_input_returns_empty_vec() {
        let splitter = MarkdownSplitter::new(2000, None);
        assert_eq!(splitter.split(""), Vec::<String>::new());
        assert_eq!(splitter.split("   \n\n  "), Vec::<String>::new());
    }

    #[test]
    fn markdown_close_and_reopen_inside_block_fence() {
        let splitter = MarkdownSplitter::new(30, None);
        let code = "x".repeat(80);
        let text = format!("```\n{code}\n```");
        let chunks = splitter.split(&text);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            let opens = c.matches("```").count();
            assert!(opens >= 2, "chunk must contain both open and close: {c}");
        }
    }

    #[test]
    fn markdown_backtracks_past_inline_backtick_span() {
        let splitter = MarkdownSplitter::new(20, None);
        let text = "prefix `inline span` suffix";
        let chunks = splitter.split(text);
        for c in &chunks {
            let open_runs: Vec<_> = c.match_indices('`').collect();
            assert!(
                open_runs.len() % 2 == 0,
                "chunk has unbalanced backticks: {c}"
            );
        }
    }

    #[test]
    fn markdown_preserves_tilde_fence_across_split() {
        let splitter = MarkdownSplitter::new(40, None);
        let code = "y".repeat(80);
        let text = format!("~~~\n{code}\n~~~");
        let chunks = splitter.split(&text);
        for c in &chunks {
            assert!(c.contains("~~~"), "tilde fence missing in chunk: {c}");
        }
    }

    #[test]
    fn telegram_v2_never_splits_after_lone_backslash() {
        let splitter = TelegramMarkdownV2Splitter::new(10, None);
        let text = "abcdef\\.gh more text after".to_string();
        let chunks = splitter.split(&text);
        for c in &chunks {
            assert!(
                !ends_with_unbalanced_backslash(c),
                "chunk ends mid-escape: {c:?}"
            );
        }
    }

    fn ends_with_unbalanced_backslash(s: &str) -> bool {
        let bytes = s.as_bytes();
        let mut count = 0;
        let mut i = bytes.len();
        while i > 0 && bytes[i - 1] == b'\\' {
            count += 1;
            i -= 1;
        }
        count % 2 == 1
    }

    #[test]
    fn telegram_v2_double_backslash_is_safe_to_split_after() {
        let raw = "abcd\\\\efghij more".to_string();
        assert_eq!(avoid_split_after_backslash(&raw, 6), 6);
    }

    #[test]
    fn telegram_v2_inherits_block_fence_close_reopen() {
        let splitter = TelegramMarkdownV2Splitter::new(30, None);
        let code = "x".repeat(80);
        let text = format!("```\n{code}\n```");
        let chunks = splitter.split(&text);
        for c in &chunks {
            let opens = c.matches("```").count();
            assert!(opens >= 2, "each chunk must be a complete code block: {c}");
        }
    }

    #[test]
    fn signal_returns_balanced_ranges_per_chunk() {
        let splitter = SignalSplitter::new(MarkdownSplitter::new(30, None));
        let text = "**hello** there\n\n**world** howdy\n\n**end** of text here";
        let signals = splitter.split(text);
        assert!(signals.len() >= 2);
        for s in &signals {
            let utf16_len = s.body.encode_utf16().count();
            for r in &s.ranges {
                let end = (r.start + r.length) as usize;
                assert!(
                    end <= utf16_len,
                    "range past body end: start={} length={} body_len={}",
                    r.start, r.length, utf16_len
                );
            }
        }
    }

    #[test]
    fn signal_cross_boundary_bold_degrades_to_plain_text() {
        let splitter = SignalSplitter::new(MarkdownSplitter::new(20, None));
        let text = "**bold across\n\nboundary** end";
        let signals = splitter.split(text);
        assert!(signals.len() >= 2);
        let total_bold_ranges: usize = signals
            .iter()
            .flat_map(|s| &s.ranges)
            .filter(|r| matches!(r.style, super::super::markdown::SignalStyle::Bold))
            .count();
        assert_eq!(total_bold_ranges, 0);
    }

    #[tokio::test]
    async fn ensure_chat_share_pre_cancelled_token_returns_transient() {
        let svc = test_share_service().await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = ensure_chat_share(&svc, "https://app.host", 3600, &cancel, "chat-1", "user-1")
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("cancelled") || format!("{err}").contains("cancelled"));
    }

    #[test]
    fn fence_depth_block_fence_wins_over_inline_scanning() {
        // Inside a block fence we never scan inline backticks, so the
        // `should not count` run must NOT toggle `inline_open`.
        let text = "```\n`should not count`\n";
        let state = fence_depth_at(text, text.len());
        assert!(state.block.is_some());
        assert_eq!(state.inline_open, None);
    }
}
