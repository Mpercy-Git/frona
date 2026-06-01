use crate::aria::node::{AriaChecked, AriaChild, AriaNode, AriaPressed};
use crate::aria::yaml::yaml_scalar;

#[derive(Debug, Clone, Copy)]
pub(crate) enum RenderMode {
    Ai,
    Compact,
}

fn compact(rendered: &str) -> String {
    let lines: Vec<&str> = rendered.lines().collect();
    let indents: Vec<usize> = lines
        .iter()
        .map(|l| l.chars().take_while(|c| *c == ' ').count())
        .collect();
    let mut keep = vec![false; lines.len()];

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let actionable = line.contains("[index=") || trimmed.contains(": ");
        if actionable {
            keep[i] = true;
            let mut min_indent = indents[i];
            for j in (0..i).rev() {
                if indents[j] < min_indent {
                    keep[j] = true;
                    min_indent = indents[j];
                    if min_indent == 0 {
                        break;
                    }
                }
            }
        }
    }

    lines
        .iter()
        .zip(keep.iter())
        .filter(|(_, k)| **k)
        .map(|(l, _)| *l)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_aria_tree(
    root: &AriaNode,
    mode: RenderMode,
    previous: Option<&AriaNode>,
) -> String {
    if matches!(mode, RenderMode::Compact) {
        let full = render_aria_tree(root, RenderMode::Ai, previous);
        return compact(&full);
    }
    let mut lines = Vec::new();
    let render_cursor_pointer = true;
    let render_active = true;

    let nodes_to_render = if root.role == "fragment" {
        &root.children
    } else {
        return render_single_node(root, mode, previous);
    };

    for node in nodes_to_render {
        match node {
            AriaChild::Text(text) => {
                visit_text(text, "", &mut lines);
            }
            AriaChild::Node(node) => {
                visit(
                    node,
                    "",
                    render_cursor_pointer,
                    render_active,
                    &mut lines,
                    previous,
                );
            }
        }
    }

    lines.join("\n")
}

fn render_single_node(root: &AriaNode, _mode: RenderMode, previous: Option<&AriaNode>) -> String {
    let mut lines = Vec::new();
    visit(root, "", true, true, &mut lines, previous);
    lines.join("\n")
}

fn visit_text(text: &str, indent: &str, lines: &mut Vec<String>) {
    let escaped = yaml_scalar(text);
    if !escaped.is_empty() {
        lines.push(format!("{indent}- text: {escaped}"));
    }
}

fn visit(
    aria_node: &AriaNode,
    indent: &str,
    render_cursor_pointer: bool,
    render_active: bool,
    lines: &mut Vec<String>,
    _previous: Option<&AriaNode>,
) {
    let key = create_key(aria_node, render_cursor_pointer, render_active);
    let escaped_key = format!("{indent}- {}", yaml_scalar(&key));

    let single_text_child = get_single_inlined_text_child(aria_node);

    if aria_node.children.is_empty() && aria_node.props.is_empty() {
        lines.push(escaped_key);
    } else if let Some(text) = single_text_child {
        lines.push(format!(
            "{escaped_key}: {}",
            yaml_scalar(&text)
        ));
    } else {
        lines.push(format!("{escaped_key}:"));
        for (name, value) in &aria_node.props {
            lines.push(format!(
                "{indent}  - /{name}: {}",
                yaml_scalar(value)
            ));
        }

        let child_indent = format!("{indent}  ");
        let in_cursor_pointer =
            aria_node.index.is_some() && render_cursor_pointer && aria_node.has_pointer_cursor();

        for child in &aria_node.children {
            match child {
                AriaChild::Text(text) => {
                    visit_text(text, &child_indent, lines);
                }
                AriaChild::Node(child_node) => {
                    visit(
                        child_node,
                        &child_indent,
                        render_cursor_pointer && !in_cursor_pointer,
                        render_active,
                        lines,
                        None,
                    );
                }
            }
        }
    }
}

fn create_key(aria_node: &AriaNode, render_cursor_pointer: bool, render_active: bool) -> String {
    let mut key = aria_node.role.clone();

    if !aria_node.name.is_empty() && aria_node.name.len() <= 900 {
        key.push(' ');
        key.push_str(&format!("{:?}", aria_node.name));
    }

    if let Some(checked) = &aria_node.checked {
        match checked {
            AriaChecked::Bool(true) => key.push_str(" [checked]"),
            AriaChecked::Bool(false) => {}
            AriaChecked::Mixed(_) => key.push_str(" [checked=mixed]"),
        }
    }

    if aria_node.disabled == Some(true) {
        key.push_str(" [disabled]");
    }

    if aria_node.expanded == Some(true) {
        key.push_str(" [expanded]");
    }

    if render_active && aria_node.active == Some(true) {
        key.push_str(" [active]");
    }

    if let Some(level) = aria_node.level {
        key.push_str(&format!(" [level={level}]"));
    }

    if let Some(pressed) = &aria_node.pressed {
        match pressed {
            AriaPressed::Bool(true) => key.push_str(" [pressed]"),
            AriaPressed::Bool(false) => {}
            AriaPressed::Mixed(_) => key.push_str(" [pressed=mixed]"),
        }
    }

    if aria_node.selected == Some(true) {
        key.push_str(" [selected]");
    }

    if let Some(index) = aria_node.index {
        key.push_str(&format!(" [index={index}]"));

        if render_cursor_pointer && aria_node.has_pointer_cursor() {
            key.push_str(" [cursor=pointer]");
        }
    }

    key
}

fn get_single_inlined_text_child(aria_node: &AriaNode) -> Option<String> {
    if aria_node.children.len() == 1
        && aria_node.props.is_empty()
        && let AriaChild::Text(text) = &aria_node.children[0]
    {
        return Some(text.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_simple_tree() {
        let mut root = AriaNode::fragment();
        root.children.push(AriaChild::Node(Box::new(
            AriaNode::new("button", "Click me")
                .with_index(0)
                .with_box(true, Some("pointer".to_string())),
        )));

        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert!(yaml.contains("button"));
        assert!(yaml.contains("Click me"));
        assert!(yaml.contains("[index=0]"));
        assert!(yaml.contains("[cursor=pointer]"));
    }

    #[test]
    fn test_render_tree_with_text() {
        let mut root = AriaNode::fragment();
        root.children
            .push(AriaChild::Text("Hello world".to_string()));

        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert!(yaml.contains("text:"));
        assert!(yaml.contains("Hello world"));
    }

    #[test]
    fn test_render_nested_tree() {
        let mut root = AriaNode::fragment();
        let mut div = AriaNode::new("generic", "");
        div.children
            .push(AriaChild::Text("Parent text".to_string()));
        div.children.push(AriaChild::Node(Box::new(
            AriaNode::new("button", "Child button").with_index(0),
        )));

        root.children.push(AriaChild::Node(Box::new(div)));

        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert!(yaml.contains("generic"));
        assert!(yaml.contains("Parent text"));
        assert!(yaml.contains("button"));
        assert!(yaml.contains("Child button"));
    }

    #[test]
    fn test_render_with_props() {
        let mut root = AriaNode::fragment();
        root.children.push(AriaChild::Node(Box::new(
            AriaNode::new("link", "Go to page")
                .with_index(0)
                .with_prop("url", "https://example.com"),
        )));

        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert!(yaml.contains("link"));
        assert!(yaml.contains("[index=0]"));
        assert!(yaml.contains("/url:"));
        assert!(yaml.contains("https://example.com"));
    }

    #[test]
    fn test_render_with_aria_states() {
        let mut root = AriaNode::fragment();
        root.children.push(AriaChild::Node(Box::new(
            AriaNode::new("checkbox", "Accept terms")
                .with_index(0)
                .with_checked(true)
                .with_disabled(false),
        )));

        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert!(yaml.contains("checkbox"));
        assert!(yaml.contains("[checked]"));
        assert!(!yaml.contains("[disabled]"));
    }

    #[test]
    fn test_render_heading_with_level() {
        let mut root = AriaNode::fragment();
        root.children.push(AriaChild::Node(Box::new(
            AriaNode::new("heading", "Page Title").with_level(1),
        )));

        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert!(yaml.contains("heading"));
        assert!(yaml.contains("Page Title"));
        assert!(yaml.contains("[level=1]"));
    }

    #[test]
    fn test_empty_snapshot() {
        let root = AriaNode::fragment();
        let yaml = render_aria_tree(&root, RenderMode::Ai, None);
        assert_eq!(yaml.trim(), "");
    }

    #[test]
    fn test_compact_keeps_indexed_and_ancestors_drops_others() {
        let mut root = AriaNode::fragment();
        let mut form = AriaNode::new("group", "");
        form.children
            .push(AriaChild::Node(Box::new(AriaNode::new("paragraph", ""))));
        form.children.push(AriaChild::Node(Box::new(
            AriaNode::new("button", "Submit").with_index(0),
        )));
        root.children.push(AriaChild::Node(Box::new(form)));
        let full = render_aria_tree(&root, RenderMode::Ai, None);
        let compact_out = render_aria_tree(&root, RenderMode::Compact, None);
        assert!(full.contains("paragraph"));
        assert!(compact_out.contains("Submit"));
        assert!(compact_out.contains("group"));
        assert!(!compact_out.contains("paragraph"));
    }
}
