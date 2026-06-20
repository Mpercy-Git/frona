use std::borrow::Cow;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Wrap GFM tables in ``` fences so platforms without native table support
/// (Telegram, Discord, …) display them as aligned monospace blocks instead of
/// ragged pipe text. Parses with the same parser+options as
/// `telegram_markdown_v2`, so the spans we wrap match what the converter
/// would treat as a table.
pub(super) fn fence_tables(text: &str) -> Cow<'_, str> {
    let Ok(::markdown::mdast::Node::Root(root)) =
        ::markdown::to_mdast(text, &::markdown::ParseOptions::gfm())
    else {
        return Cow::Borrowed(text);
    };

    let ranges: Vec<(usize, usize)> = root
        .children
        .iter()
        .filter_map(|node| match node {
            ::markdown::mdast::Node::Table(t) => {
                t.position.as_ref().map(|p| (p.start.offset, p.end.offset))
            }
            _ => None,
        })
        .collect();

    if ranges.is_empty() {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len() + ranges.len() * 8);
    let mut cursor = 0;
    for (start, end) in ranges {
        let span = &text[start..end];
        // Re-render the span through the converter's own table renderer
        // (Keep = no escaping): its width-padding math aligns the columns
        // regardless of how the model padded its cells.
        let aligned = telegram_markdown_v2::convert_with_strategy(
            span,
            telegram_markdown_v2::UnsupportedTagsStrategy::Keep,
        )
        .unwrap_or_else(|_| span.to_string());
        out.push_str(&text[cursor..start]);
        out.push_str("```\n");
        out.push_str(aligned.trim_end_matches('\n'));
        out.push_str("\n```");
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    Cow::Owned(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalStyle {
    Bold,
    Italic,
    Strikethrough,
    Monospace,
}

/// `start` and `length` are **UTF-16 code units**, not bytes or chars; Signal's
/// protocol requires this. Emoji and non-BMP characters take 2 units each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalBodyRange {
    pub start: u32,
    pub length: u32,
    pub style: SignalStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalText {
    pub body: String,
    pub ranges: Vec<SignalBodyRange>,
}

/// Convert markdown to Signal's plain-text + `BodyRange` representation.
/// Links flatten to `text (url)` (Signal autolinks raw URLs); headings,
/// blockquotes, and lists render as plain text since Signal has no
/// equivalent styles.
pub fn to_signal(input: &str) -> SignalText {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(input, opts);

    let mut out = String::with_capacity(input.len());
    let mut ranges: Vec<SignalBodyRange> = Vec::new();
    let mut style_stack: Vec<(SignalStyle, u32)> = Vec::new();
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut link_text_buf: Option<String> = None;
    let mut link_dest: Option<String> = None;
    let mut in_code_block = false;
    let mut code_block_start: u32 = 0;
    let mut at_line_start = true;
    let mut pending_blank_line = false;

    // Tracked incrementally; recomputing on every push would be O(n²).
    let mut utf16_len: u32 = 0;

    fn push_str(
        out: &mut String,
        utf16_len: &mut u32,
        at_line_start: &mut bool,
        pending_blank_line: &mut bool,
        s: &str,
    ) {
        if s.is_empty() {
            return;
        }
        if *pending_blank_line {
            if !out.is_empty() && !out.ends_with("\n\n") {
                if out.ends_with('\n') {
                    out.push('\n');
                    *utf16_len += 1;
                } else {
                    out.push_str("\n\n");
                    *utf16_len += 2;
                }
            }
            *pending_blank_line = false;
        }
        out.push_str(s);
        *utf16_len = utf16_len.saturating_add(s.encode_utf16().count() as u32);
        *at_line_start = s.ends_with('\n');
    }

    fn break_line(out: &mut String, utf16_len: &mut u32, at_line_start: &mut bool) {
        if !*at_line_start {
            out.push('\n');
            *utf16_len += 1;
            *at_line_start = true;
        }
    }

    /// Call before any direct `out.push_str(...)` that bypasses `push_str`,
    /// otherwise the deferred blank line lands AFTER the write and orphans
    /// the bullet on its own line.
    fn flush_pending_blank(
        out: &mut String,
        utf16_len: &mut u32,
        pending_blank_line: &mut bool,
    ) {
        if !*pending_blank_line {
            return;
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            if out.ends_with('\n') {
                out.push('\n');
                *utf16_len += 1;
            } else {
                out.push_str("\n\n");
                *utf16_len += 2;
            }
        }
        *pending_blank_line = false;
    }

    for event in parser {
        match event {
            Event::Text(t) => {
                if let Some(buf) = link_text_buf.as_mut() {
                    buf.push_str(&t);
                } else {
                    push_str(
                        &mut out,
                        &mut utf16_len,
                        &mut at_line_start,
                        &mut pending_blank_line,
                        &t,
                    );
                }
            }
            Event::Code(t) => {
                if let Some(buf) = link_text_buf.as_mut() {
                    // No range inside a link: the rendered string changes
                    // when we resolve `text (url)` so offsets would shift.
                    buf.push_str(&t);
                } else {
                    let start = utf16_len;
                    push_str(
                        &mut out,
                        &mut utf16_len,
                        &mut at_line_start,
                        &mut pending_blank_line,
                        &t,
                    );
                    let length = utf16_len.saturating_sub(start);
                    if length > 0 {
                        ranges.push(SignalBodyRange {
                            start,
                            length,
                            style: SignalStyle::Monospace,
                        });
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !at_line_start {
                    if matches!(event, Event::HardBreak) {
                        out.push('\n');
                        utf16_len += 1;
                        at_line_start = true;
                    } else {
                        out.push(' ');
                        utf16_len += 1;
                    }
                }
            }
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                }
                Tag::Heading { level, .. } => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) && !out.is_empty() {
                        pending_blank_line = true;
                    }
                }
                Tag::BlockQuote(_) => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                }
                Tag::CodeBlock(_) => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    in_code_block = true;
                    code_block_start = utf16_len;
                }
                Tag::List(start) => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    list_stack.push(start);
                }
                Tag::Item => {
                    flush_pending_blank(&mut out, &mut utf16_len, &mut pending_blank_line);
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    let depth = list_stack.len().saturating_sub(1);
                    for _ in 0..depth {
                        out.push_str("  ");
                        utf16_len += 2;
                    }
                    if let Some(top) = list_stack.last_mut() {
                        match top {
                            Some(n) => {
                                let bullet = format!("{n}. ");
                                let bullet_utf16 = bullet.encode_utf16().count() as u32;
                                out.push_str(&bullet);
                                utf16_len += bullet_utf16;
                                *n += 1;
                            }
                            None => {
                                out.push_str("- ");
                                utf16_len += 2;
                            }
                        }
                    }
                    at_line_start = false;
                }
                Tag::Link { dest_url, .. } => {
                    link_text_buf = Some(String::new());
                    link_dest = Some(dest_url.into_string());
                }
                Tag::Image { dest_url, .. } => {
                    let _ = dest_url;
                    link_text_buf = Some(String::new());
                    link_dest = None;
                }
                Tag::Strong => {
                    style_stack.push((SignalStyle::Bold, utf16_len));
                }
                Tag::Emphasis => {
                    style_stack.push((SignalStyle::Italic, utf16_len));
                }
                Tag::Strikethrough => {
                    style_stack.push((SignalStyle::Strikethrough, utf16_len));
                }
                Tag::Superscript | Tag::Subscript | Tag::HtmlBlock
                | Tag::FootnoteDefinition(_) | Tag::DefinitionList
                | Tag::DefinitionListTitle | Tag::DefinitionListDefinition
                | Tag::Table(_) | Tag::TableHead | Tag::TableRow
                | Tag::TableCell | Tag::MetadataBlock(_) => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_) => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    pending_blank_line = true;
                }
                TagEnd::CodeBlock => {
                    let length = utf16_len.saturating_sub(code_block_start);
                    if in_code_block && length > 0 {
                        // Strip pulldown-cmark's trailing block newline.
                        let trimmed_len = if out.ends_with('\n') { length - 1 } else { length };
                        if trimmed_len > 0 {
                            ranges.push(SignalBodyRange {
                                start: code_block_start,
                                length: trimmed_len,
                                style: SignalStyle::Monospace,
                            });
                        }
                    }
                    in_code_block = false;
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    pending_blank_line = true;
                }
                TagEnd::List(_) => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                    list_stack.pop();
                    if list_stack.is_empty() {
                        pending_blank_line = true;
                    }
                }
                TagEnd::Item => {
                    break_line(&mut out, &mut utf16_len, &mut at_line_start);
                }
                TagEnd::Link => {
                    let text = link_text_buf.take().unwrap_or_default();
                    let dest = link_dest.take().unwrap_or_default();
                    let rendered = if dest.is_empty() || dest == text {
                        text
                    } else {
                        format!("{text} ({dest})")
                    };
                    push_str(
                        &mut out,
                        &mut utf16_len,
                        &mut at_line_start,
                        &mut pending_blank_line,
                        &rendered,
                    );
                }
                TagEnd::Image => {
                    let alt = link_text_buf.take().unwrap_or_default();
                    push_str(
                        &mut out,
                        &mut utf16_len,
                        &mut at_line_start,
                        &mut pending_blank_line,
                        &alt,
                    );
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    let expected_style = match tag_end {
                        TagEnd::Strong => SignalStyle::Bold,
                        TagEnd::Emphasis => SignalStyle::Italic,
                        TagEnd::Strikethrough => SignalStyle::Strikethrough,
                        _ => unreachable!(),
                    };
                    if let Some((style, start)) = style_stack.pop() {
                        debug_assert_eq!(style, expected_style);
                        let length = utf16_len.saturating_sub(start);
                        if length > 0 {
                            ranges.push(SignalBodyRange { start, length, style });
                        }
                    }
                }
                _ => {}
            },
            Event::Rule
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
        }
    }

    // Collapse 3+ newlines to 2 while building a delta table that maps
    // pre-collapse UTF-16 offsets to post-collapse offsets, so ranges
    // collected above can be shifted into the trimmed body.
    let trimmed = out.trim_end();
    let trimmed_owned = trimmed.to_string();
    let mut body = String::with_capacity(trimmed_owned.len());
    let mut consecutive_newlines = 0u8;
    let mut deltas: Vec<i32> = Vec::with_capacity(trimmed_owned.encode_utf16().count());
    let mut running_delta: i32 = 0;
    for ch in trimmed_owned.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                body.push(ch);
                deltas.push(running_delta);
            } else {
                running_delta -= 1;
                deltas.push(running_delta);
            }
        } else {
            consecutive_newlines = 0;
            body.push(ch);
            for _ in 0..ch.len_utf16() {
                deltas.push(running_delta);
            }
        }
    }

    let final_utf16_len = body.encode_utf16().count() as u32;

    let ranges = ranges
        .into_iter()
        .filter_map(|r| {
            let new_start = (r.start as i32) + deltas.get(r.start as usize).copied().unwrap_or(0);
            let end_old = r.start.saturating_add(r.length);
            let new_end = (end_old as i32)
                + deltas
                    .get(end_old.saturating_sub(1) as usize)
                    .copied()
                    .unwrap_or(running_delta);
            let new_start = new_start.max(0) as u32;
            let new_end = (new_end.max(0) as u32).min(final_utf16_len);
            if new_end > new_start {
                Some(SignalBodyRange {
                    start: new_start,
                    length: new_end - new_start,
                    style: r.style,
                })
            } else {
                None
            }
        })
        .collect();

    SignalText { body, ranges }
}

/// Convert markdown to WhatsApp's formatting flavor:
/// - `**bold**` / `__bold__` → `*bold*`
/// - `*italic*` / `_italic_` → `_italic_`
/// - `~~strike~~` → `~strike~`
/// - inline `` `code` `` and fenced blocks → kept as-is
/// - links → `text: url` when the label differs from the URL, else just the URL
///   (WhatsApp auto-linkifies plain URLs in chat)
/// - headings → wrapped in `*bold*` since WhatsApp has no heading syntax
/// - lists, blockquotes → `* `/`1. ` items, `> ` quote prefix
pub fn to_whatsapp(input: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(input, opts);
    let mut out = String::with_capacity(input.len());
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut link_text_buf: Option<String> = None;
    let mut link_dest: Option<String> = None;
    let mut at_line_start = true;
    let mut pending_blank_line = false;

    fn push(
        out: &mut String,
        at_line_start: &mut bool,
        pending_blank_line: &mut bool,
        s: &str,
    ) {
        if s.is_empty() {
            return;
        }
        if *pending_blank_line {
            if !out.is_empty() && !out.ends_with("\n\n") {
                if out.ends_with('\n') {
                    out.push('\n');
                } else {
                    out.push_str("\n\n");
                }
            }
            *pending_blank_line = false;
        }
        out.push_str(s);
        *at_line_start = s.ends_with('\n');
    }
    fn break_line(out: &mut String, at_line_start: &mut bool) {
        if !*at_line_start {
            out.push('\n');
            *at_line_start = true;
        }
    }

    for event in parser {
        match event {
            Event::Text(t) => {
                if let Some(buf) = link_text_buf.as_mut() {
                    buf.push_str(&t);
                } else {
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, &t);
                }
            }
            Event::Code(t) => {
                let wrapped = format!("`{t}`");
                if let Some(buf) = link_text_buf.as_mut() {
                    buf.push_str(&wrapped);
                } else {
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, &wrapped);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !at_line_start {
                    out.push(if matches!(event, Event::HardBreak) { '\n' } else { ' ' });
                    at_line_start = matches!(event, Event::HardBreak);
                }
            }
            Event::Start(tag) => match tag {
                Tag::Paragraph => break_line(&mut out, &mut at_line_start),
                Tag::Heading { level, .. } => {
                    break_line(&mut out, &mut at_line_start);
                    if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) && !out.is_empty() {
                        pending_blank_line = true;
                    }
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, "*");
                }
                Tag::BlockQuote(_) => {
                    break_line(&mut out, &mut at_line_start);
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, "> ");
                }
                Tag::CodeBlock(_) => {
                    break_line(&mut out, &mut at_line_start);
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, "```\n");
                }
                Tag::List(start) => {
                    break_line(&mut out, &mut at_line_start);
                    list_stack.push(start);
                }
                Tag::Item => {
                    break_line(&mut out, &mut at_line_start);
                    let depth = list_stack.len().saturating_sub(1);
                    for _ in 0..depth {
                        out.push_str("  ");
                    }
                    if let Some(top) = list_stack.last_mut() {
                        match top {
                            Some(n) => {
                                out.push_str(&format!("{n}. "));
                                *n += 1;
                            }
                            None => out.push_str("* "),
                        }
                    }
                    at_line_start = false;
                }
                Tag::Link { dest_url, .. } => {
                    link_text_buf = Some(String::new());
                    link_dest = Some(dest_url.into_string());
                }
                Tag::Image { dest_url, .. } => {
                    let _ = dest_url;
                    link_text_buf = Some(String::new());
                    link_dest = None;
                }
                Tag::Strong => push(&mut out, &mut at_line_start, &mut pending_blank_line, "*"),
                Tag::Emphasis => push(&mut out, &mut at_line_start, &mut pending_blank_line, "_"),
                Tag::Strikethrough => push(&mut out, &mut at_line_start, &mut pending_blank_line, "~"),
                Tag::Superscript | Tag::Subscript | Tag::HtmlBlock
                | Tag::FootnoteDefinition(_) | Tag::DefinitionList
                | Tag::DefinitionListTitle | Tag::DefinitionListDefinition
                | Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell
                | Tag::MetadataBlock(_) => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph | TagEnd::BlockQuote(_) => {
                    break_line(&mut out, &mut at_line_start);
                    pending_blank_line = true;
                }
                TagEnd::Heading(_) => {
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, "*");
                    break_line(&mut out, &mut at_line_start);
                    pending_blank_line = true;
                }
                TagEnd::CodeBlock => {
                    break_line(&mut out, &mut at_line_start);
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, "```");
                    break_line(&mut out, &mut at_line_start);
                    pending_blank_line = true;
                }
                TagEnd::List(_) => {
                    break_line(&mut out, &mut at_line_start);
                    list_stack.pop();
                    if list_stack.is_empty() {
                        pending_blank_line = true;
                    }
                }
                TagEnd::Item => break_line(&mut out, &mut at_line_start),
                TagEnd::Link => {
                    let text = link_text_buf.take().unwrap_or_default();
                    let dest = link_dest.take().unwrap_or_default();
                    let rendered = if dest.is_empty() || dest == text {
                        text
                    } else if text.is_empty() {
                        dest
                    } else {
                        format!("{text}: {dest}")
                    };
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, &rendered);
                }
                TagEnd::Image => {
                    let alt = link_text_buf.take().unwrap_or_default();
                    push(&mut out, &mut at_line_start, &mut pending_blank_line, &alt);
                }
                TagEnd::Strong => push(&mut out, &mut at_line_start, &mut pending_blank_line, "*"),
                TagEnd::Emphasis => push(&mut out, &mut at_line_start, &mut pending_blank_line, "_"),
                TagEnd::Strikethrough => push(&mut out, &mut at_line_start, &mut pending_blank_line, "~"),
                _ => {}
            },
            Event::Rule
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
        }
    }

    // Drop empty-bullet lines (e.g. `* ` with no payload) - WhatsApp would
    // otherwise render them as lone asterisks.
    let stripped: String = out
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed == "*" || trimmed == "-" || trimmed.is_empty() && !line.is_empty()
                || (trimmed.ends_with('.')
                    && trimmed[..trimmed.len() - 1].chars().all(|c| c.is_ascii_digit())))
        })
        .collect::<Vec<_>>()
        .join("\n");

    let trimmed = stripped.trim_end();
    let mut collapsed = String::with_capacity(trimmed.len());
    let mut consecutive_newlines = 0u8;
    for ch in trimmed.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                collapsed.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            collapsed.push(ch);
        }
    }
    collapsed
}

pub fn to_plain(input: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(input, opts);
    let mut out = String::with_capacity(input.len());
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut link_text_buf: Option<String> = None;
    let mut link_dest: Option<String> = None;
    let mut at_line_start = true;
    let mut pending_blank_line = false;

    let push_str = |out: &mut String,
                    at_line_start: &mut bool,
                    pending_blank_line: &mut bool,
                    s: &str| {
        if s.is_empty() {
            return;
        }
        if *pending_blank_line {
            if !out.is_empty() && !out.ends_with("\n\n") {
                if out.ends_with('\n') {
                    out.push('\n');
                } else {
                    out.push_str("\n\n");
                }
            }
            *pending_blank_line = false;
        }
        out.push_str(s);
        *at_line_start = s.ends_with('\n');
    };

    let break_line = |out: &mut String, at_line_start: &mut bool| {
        if !*at_line_start {
            out.push('\n');
            *at_line_start = true;
        }
    };

    for event in parser {
        match event {
            Event::Text(t) => {
                if let Some(buf) = link_text_buf.as_mut() {
                    buf.push_str(&t);
                } else {
                    push_str(&mut out, &mut at_line_start, &mut pending_blank_line, &t);
                }
            }
            Event::Code(t) => {
                if let Some(buf) = link_text_buf.as_mut() {
                    buf.push_str(&t);
                } else {
                    push_str(&mut out, &mut at_line_start, &mut pending_blank_line, &t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !at_line_start {
                    out.push(if matches!(event, Event::HardBreak) { '\n' } else { ' ' });
                    at_line_start = matches!(event, Event::HardBreak);
                }
            }
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    break_line(&mut out, &mut at_line_start);
                }
                Tag::Heading { level, .. } => {
                    break_line(&mut out, &mut at_line_start);
                    if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) && !out.is_empty() {
                        pending_blank_line = true;
                    }
                }
                Tag::BlockQuote(_) => {
                    break_line(&mut out, &mut at_line_start);
                }
                Tag::CodeBlock(_) => {
                    break_line(&mut out, &mut at_line_start);
                }
                Tag::List(start) => {
                    break_line(&mut out, &mut at_line_start);
                    list_stack.push(start);
                }
                Tag::Item => {
                    break_line(&mut out, &mut at_line_start);
                    let depth = list_stack.len().saturating_sub(1);
                    for _ in 0..depth {
                        out.push_str("  ");
                    }
                    if let Some(top) = list_stack.last_mut() {
                        match top {
                            Some(n) => {
                                out.push_str(&format!("{n}. "));
                                *n += 1;
                            }
                            None => out.push_str("- "),
                        }
                    }
                    at_line_start = false;
                }
                Tag::Link { dest_url, .. } => {
                    link_text_buf = Some(String::new());
                    link_dest = Some(dest_url.into_string());
                }
                Tag::Image { dest_url, .. } => {
                    let _ = dest_url;
                    link_text_buf = Some(String::new());
                    link_dest = None;
                }
                Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript
                | Tag::Subscript | Tag::HtmlBlock | Tag::FootnoteDefinition(_)
                | Tag::DefinitionList | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition | Tag::Table(_) | Tag::TableHead
                | Tag::TableRow | Tag::TableCell | Tag::MetadataBlock(_) => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock => {
                    break_line(&mut out, &mut at_line_start);
                    pending_blank_line = true;
                }
                TagEnd::List(_) => {
                    break_line(&mut out, &mut at_line_start);
                    list_stack.pop();
                    if list_stack.is_empty() {
                        pending_blank_line = true;
                    }
                }
                TagEnd::Item => {
                    break_line(&mut out, &mut at_line_start);
                }
                TagEnd::Link => {
                    let text = link_text_buf.take().unwrap_or_default();
                    let dest = link_dest.take().unwrap_or_default();
                    let rendered = if dest.is_empty() || dest == text {
                        text
                    } else {
                        format!("{text} ({dest})")
                    };
                    push_str(
                        &mut out,
                        &mut at_line_start,
                        &mut pending_blank_line,
                        &rendered,
                    );
                }
                TagEnd::Image => {
                    let alt = link_text_buf.take().unwrap_or_default();
                    push_str(&mut out, &mut at_line_start, &mut pending_blank_line, &alt);
                }
                _ => {}
            },
            Event::Rule
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
        }
    }

    let trimmed = out.trim_end();
    let mut collapsed = String::with_capacity(trimmed.len());
    let mut consecutive_newlines = 0u8;
    for ch in trimmed.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                collapsed.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            collapsed.push(ch);
        }
    }
    collapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_tables_wraps_top_level_table() {
        let input = "Intro\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nOutro";
        let fenced = fence_tables(input);
        assert_eq!(
            fenced,
            "Intro\n\n```\n| a | b |\n| - | - |\n| 1 | 2 |\n```\n\nOutro"
        );
    }

    #[test]
    fn fence_tables_realigns_ragged_columns() {
        let input = "| Col A | B |\n| - | - |\n| 1 | 2 |";
        let fenced = fence_tables(input);
        assert_eq!(fenced, "```\n| Col A | B |\n| -     | - |\n| 1     | 2 |\n```");
    }

    #[test]
    fn fence_tables_leaves_plain_text_untouched() {
        let input = "no tables here, just a | pipe";
        assert!(matches!(fence_tables(input), Cow::Borrowed(_)));
    }

    #[test]
    fn fence_tables_skips_tables_already_inside_code_fences() {
        let input = "```\n| a | b |\n| - | - |\n```\n";
        assert!(matches!(fence_tables(input), Cow::Borrowed(_)));
    }

    #[test]
    fn wa_bold_uses_single_asterisk() {
        assert_eq!(to_whatsapp("**Navigation**"), "*Navigation*");
    }

    #[test]
    fn wa_italic_uses_underscore() {
        assert_eq!(to_whatsapp("*italic* and _also_"), "_italic_ and _also_");
    }

    #[test]
    fn wa_strikethrough_uses_single_tilde() {
        assert_eq!(to_whatsapp("~~gone~~"), "~gone~");
    }

    #[test]
    fn wa_empty_bullet_is_dropped() {
        // Regression: agent output with a stray empty bullet must not leave a
        // `*` on its own line in WhatsApp (the screenshot bug from May 2026).
        let md = "**Heading**\n\n* \n\n* real item";
        let out = to_whatsapp(md);
        for line in out.lines() {
            let t = line.trim();
            assert!(t != "*" && t != "* ", "stray bullet line in output: {out:?}");
        }
        assert!(out.contains("real item"), "real-item text missing: {out:?}");
        assert!(out.contains("*Heading*"), "heading missing: {out:?}");
    }

    #[test]
    fn wa_tight_list_keeps_bullets() {
        // Tight lists (no blank lines between items) should keep their bullets
        // attached to each item.
        let out = to_whatsapp("* one\n* two\n* three");
        assert!(out.contains("* one"), "tight bullet one missing: {out:?}");
        assert!(out.contains("* two"), "tight bullet two missing: {out:?}");
        assert!(out.contains("* three"), "tight bullet three missing: {out:?}");
    }

    #[test]
    fn wa_link_with_distinct_url_renders_text_and_url() {
        assert_eq!(to_whatsapp("see [docs](https://x.com)"), "see docs: https://x.com");
    }

    #[test]
    fn plain_passes_through() {
        assert_eq!(to_plain("hello world"), "hello world");
    }

    #[test]
    fn strips_emphasis() {
        assert_eq!(to_plain("**bold** and *italic* and ~~strike~~"), "bold and italic and strike");
    }

    #[test]
    fn inline_code_keeps_content_no_backticks() {
        assert_eq!(to_plain("call `fn()` to do it"), "call fn() to do it");
    }

    #[test]
    fn fenced_code_block_preserves_content() {
        let md = "```\nlet x = 1;\n```";
        assert_eq!(to_plain(md), "let x = 1;");
    }

    #[test]
    fn link_with_distinct_url_renders_text_and_url() {
        let md = "see [docs](https://example.com/d)";
        assert_eq!(to_plain(md), "see docs (https://example.com/d)");
    }

    #[test]
    fn link_with_matching_url_drops_url() {
        let md = "[https://example.com](https://example.com)";
        assert_eq!(to_plain(md), "https://example.com");
    }

    #[test]
    fn bullet_list_preserves_dashes() {
        let md = "- one\n- two\n- three";
        assert_eq!(to_plain(md), "- one\n- two\n- three");
    }

    #[test]
    fn ordered_list_keeps_numbers() {
        let md = "1. first\n2. second";
        assert_eq!(to_plain(md), "1. first\n2. second");
    }

    #[test]
    fn heading_keeps_text_drops_hash() {
        let md = "# Title\n\nbody text";
        assert_eq!(to_plain(md), "Title\n\nbody text");
    }

    #[test]
    fn blockquote_drops_marker() {
        let md = "> quoted line";
        assert_eq!(to_plain(md), "quoted line");
    }

    #[test]
    fn horizontal_rule_disappears() {
        let md = "above\n\n---\n\nbelow";
        assert_eq!(to_plain(md), "above\n\nbelow");
    }

    #[test]
    fn nested_emphasis_collapses_correctly() {
        assert_eq!(to_plain("***strong italic***"), "strong italic");
    }

    #[test]
    fn paragraph_breaks_become_blank_lines() {
        let md = "first paragraph\n\nsecond paragraph";
        assert_eq!(to_plain(md), "first paragraph\n\nsecond paragraph");
    }

    // ─── to_signal ────────────────────────────────────────────────────────

    fn r(start: u32, length: u32, style: SignalStyle) -> SignalBodyRange {
        SignalBodyRange { start, length, style }
    }

    #[test]
    fn signal_plain_text_has_no_ranges() {
        let out = to_signal("hello world");
        assert_eq!(out.body, "hello world");
        assert!(out.ranges.is_empty());
    }

    #[test]
    fn signal_bold_strips_asterisks_and_emits_range() {
        let out = to_signal("**hello** world");
        assert_eq!(out.body, "hello world");
        assert_eq!(out.ranges, vec![r(0, 5, SignalStyle::Bold)]);
    }

    #[test]
    fn signal_italic_strips_asterisks() {
        let out = to_signal("*hello* world");
        assert_eq!(out.body, "hello world");
        assert_eq!(out.ranges, vec![r(0, 5, SignalStyle::Italic)]);
    }

    #[test]
    fn signal_strikethrough() {
        let out = to_signal("~~gone~~");
        assert_eq!(out.body, "gone");
        assert_eq!(out.ranges, vec![r(0, 4, SignalStyle::Strikethrough)]);
    }

    #[test]
    fn signal_inline_code_is_monospace() {
        let out = to_signal("run `cargo test` first");
        assert_eq!(out.body, "run cargo test first");
        assert_eq!(out.ranges, vec![r(4, 10, SignalStyle::Monospace)]);
    }

    #[test]
    fn signal_nested_bold_italic_overlaps() {
        // **_both_** → BOLD over [0,4), ITALIC over [0,4) on body "both"
        let out = to_signal("**_both_**");
        assert_eq!(out.body, "both");
        // Order in `ranges` reflects close-tag order: inner italic closes
        // first, then outer bold. Both must cover "both" (length 4).
        let bold_range = out.ranges.iter().find(|r| r.style == SignalStyle::Bold);
        let italic_range = out.ranges.iter().find(|r| r.style == SignalStyle::Italic);
        assert_eq!(bold_range, Some(&r(0, 4, SignalStyle::Bold)));
        assert_eq!(italic_range, Some(&r(0, 4, SignalStyle::Italic)));
    }

    #[test]
    fn signal_fenced_code_block_is_monospace() {
        let md = "```\nlet x = 1;\n```";
        let out = to_signal(md);
        // Body contains the code lines; trailing newline trimmed by collapse.
        assert!(out.body.contains("let x = 1;"));
        let mono = out.ranges.iter().find(|r| r.style == SignalStyle::Monospace);
        assert!(mono.is_some(), "expected MONOSPACE range, got {:?}", out.ranges);
    }

    #[test]
    fn signal_link_with_text_becomes_text_parens_url() {
        let out = to_signal("see [docs](https://example.com)");
        assert_eq!(out.body, "see docs (https://example.com)");
        assert!(out.ranges.is_empty());
    }

    #[test]
    fn signal_bare_link_passes_through() {
        let out = to_signal("[https://example.com](https://example.com)");
        assert_eq!(out.body, "https://example.com");
        assert!(out.ranges.is_empty());
    }

    #[test]
    fn signal_heading_renders_as_plain_text() {
        let out = to_signal("# Title\n\nbody");
        assert_eq!(out.body, "Title\n\nbody");
        // No range - Signal has no heading style.
        assert!(out.ranges.is_empty());
    }

    #[test]
    fn signal_list_renders_with_bullets() {
        let md = "- one\n- two\n- three";
        let out = to_signal(md);
        assert_eq!(out.body, "- one\n- two\n- three");
        assert!(out.ranges.is_empty());
    }

    #[test]
    fn signal_emoji_uses_utf16_offsets() {
        // 🚀 is a non-BMP char: 2 UTF-16 units, 1 char, 4 bytes.
        // "🚀 **boom**" → body "🚀 boom"; bold over "boom" which starts at
        // UTF-16 offset 3 (rocket=2, space=1), length 4.
        let out = to_signal("🚀 **boom**");
        assert_eq!(out.body, "🚀 boom");
        assert_eq!(out.ranges, vec![r(3, 4, SignalStyle::Bold)]);
    }

    #[test]
    fn signal_empty_input_yields_empty() {
        let out = to_signal("");
        assert_eq!(out.body, "");
        assert!(out.ranges.is_empty());
    }

    #[test]
    fn signal_bold_in_middle_of_text() {
        let out = to_signal("before **mid** after");
        assert_eq!(out.body, "before mid after");
        assert_eq!(out.ranges, vec![r(7, 3, SignalStyle::Bold)]);
    }

    #[test]
    fn signal_heading_then_list_keeps_bullets_attached() {
        // Regression: deferred blank-line from heading-end used to flush
        // AFTER the `- ` bullet, orphaning the bullet and pushing the item
        // content onto an unbulleted next line.
        let md = "## Heading\n\n- one\n- two";
        let out = to_signal(md);
        assert_eq!(out.body, "Heading\n\n- one\n- two");
    }
}
