//! Slash-invocation parser. Resolution to skill vs command vs agent happens
//! in the caller, since this layer doesn't have registry access.

/// `name` is lowercased; `rest` is verbatim including internal whitespace.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedInvocation {
    Slash { name: String, rest: String },
    At { name: String, rest: String },
}

/// Returns `None` on invalid name characters rather than erroring — that's
/// what makes `/path/to/file` and `@somebody's note` fall through to plain text.
pub fn parse(content: &str) -> Option<ParsedInvocation> {
    let mut chars = content.chars();
    let prefix = chars.next()?;

    let (name_with_rest, ctor): (&str, fn(String, String) -> ParsedInvocation) = match prefix {
        '/' => (
            &content[1..],
            |name, rest| ParsedInvocation::Slash { name, rest },
        ),
        '@' => (
            &content[1..],
            |name, rest| ParsedInvocation::At { name, rest },
        ),
        _ => return None,
    };

    let split_at = name_with_rest
        .find(char::is_whitespace)
        .unwrap_or(name_with_rest.len());
    let raw_name = &name_with_rest[..split_at];

    if !is_valid_name(raw_name) {
        return None;
    }

    let rest = if split_at == name_with_rest.len() {
        String::new()
    } else {
        // Consume exactly one whitespace separator, preserve everything after.
        name_with_rest[split_at..]
            .chars()
            .next()
            .map(|c| name_with_rest[split_at + c.len_utf8()..].to_string())
            .unwrap_or_default()
    };

    Some(ctor(raw_name.to_lowercase(), rest))
}

/// `[a-z0-9][a-z0-9-_]*`.
fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// POSIX-ish shell split. Honors `"..."` and `'...'`; unbalanced open-quote
/// consumes to end-of-string rather than erroring.
pub fn shell_split(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            '\\' if in_double => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_returns_none() {
        assert!(parse("hello").is_none());
        assert!(parse("").is_none());
        assert!(parse(" /clear").is_none());
    }

    #[test]
    fn slash_with_no_args() {
        assert_eq!(
            parse("/clear"),
            Some(ParsedInvocation::Slash {
                name: "clear".to_string(),
                rest: String::new(),
            })
        );
    }

    #[test]
    fn slash_with_args() {
        assert_eq!(
            parse("/weather London"),
            Some(ParsedInvocation::Slash {
                name: "weather".to_string(),
                rest: "London".to_string(),
            })
        );
    }

    #[test]
    fn slash_preserves_internal_whitespace_in_rest() {
        assert_eq!(
            parse("/agent developer  hello  world"),
            Some(ParsedInvocation::Slash {
                name: "agent".to_string(),
                rest: "developer  hello  world".to_string(),
            })
        );
    }

    #[test]
    fn slash_lowercases_name() {
        assert_eq!(
            parse("/Weather LONDON"),
            Some(ParsedInvocation::Slash {
                name: "weather".to_string(),
                rest: "LONDON".to_string(),
            })
        );
    }

    #[test]
    fn at_resolves_to_at_variant() {
        assert_eq!(
            parse("@developer hi"),
            Some(ParsedInvocation::At {
                name: "developer".to_string(),
                rest: "hi".to_string(),
            })
        );
    }

    #[test]
    fn at_lowercases_for_case_insensitive_lookup() {
        assert_eq!(
            parse("@Developer hi"),
            Some(ParsedInvocation::At {
                name: "developer".to_string(),
                rest: "hi".to_string(),
            })
        );
    }

    #[test]
    fn name_with_hyphen_and_underscore() {
        assert_eq!(
            parse("/switch-agent_v2 x"),
            Some(ParsedInvocation::Slash {
                name: "switch-agent_v2".to_string(),
                rest: "x".to_string(),
            })
        );
    }

    #[test]
    fn rejects_path_like_text() {
        assert!(parse("/path/to/file").is_none());
    }

    #[test]
    fn rejects_name_starting_with_hyphen() {
        assert!(parse("/-foo").is_none());
    }

    #[test]
    fn empty_name_returns_none() {
        assert!(parse("/").is_none());
        assert!(parse("@").is_none());
        assert!(parse("/ ").is_none());
    }

    #[test]
    fn shell_split_basic() {
        assert_eq!(shell_split(""), Vec::<String>::new());
        assert_eq!(shell_split("a b c"), vec!["a", "b", "c"]);
        assert_eq!(shell_split("  a   b  "), vec!["a", "b"]);
    }

    #[test]
    fn shell_split_double_quotes() {
        assert_eq!(
            shell_split(r#"fix "the typo" please"#),
            vec!["fix", "the typo", "please"]
        );
    }

    #[test]
    fn shell_split_single_quotes() {
        assert_eq!(
            shell_split("say 'hello world' now"),
            vec!["say", "hello world", "now"]
        );
    }

    #[test]
    fn shell_split_mixed_quotes_preserve_inner() {
        assert_eq!(
            shell_split(r#"a "b 'c' d" e"#),
            vec!["a", "b 'c' d", "e"]
        );
    }

    #[test]
    fn shell_split_backslash_escape_in_double_quotes() {
        assert_eq!(
            shell_split(r#""he said \"hi\"""#),
            vec![r#"he said "hi""#]
        );
    }

    #[test]
    fn shell_split_unbalanced_quote_tolerated() {
        assert_eq!(shell_split(r#"a "b c"#), vec!["a", "b c"]);
    }
}
