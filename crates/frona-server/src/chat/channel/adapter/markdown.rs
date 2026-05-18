use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

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
}
