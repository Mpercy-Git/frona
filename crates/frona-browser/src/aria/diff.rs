use similar::{ChangeTag, TextDiff};

pub fn diff_snapshots(before: &str, after: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    let mut has_change = false;
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Insert => {
                has_change = true;
                "+ "
            }
            ChangeTag::Delete => {
                has_change = true;
                "- "
            }
            ChangeTag::Equal => "  ",
        };
        out.push_str(sign);
        out.push_str(change.value().trim_end_matches('\n'));
        out.push('\n');
    }
    if has_change {
        out.trim_end().to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_produce_empty_diff() {
        let s = "- button \"A\"\n- button \"B\"";
        assert_eq!(diff_snapshots(s, s), "");
    }

    #[test]
    fn additions_are_flagged() {
        let before = "- button \"A\"";
        let after = "- button \"A\"\n- button \"B\"";
        let diff = diff_snapshots(before, after);
        assert!(diff.contains("+ - button \"B\""), "got:\n{diff}");
    }

    #[test]
    fn removals_are_flagged() {
        let before = "- button \"A\"\n- button \"B\"";
        let after = "- button \"A\"";
        let diff = diff_snapshots(before, after);
        assert!(diff.contains("- - button \"B\""), "got:\n{diff}");
    }
}
