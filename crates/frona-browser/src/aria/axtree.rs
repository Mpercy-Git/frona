use std::collections::HashMap;

use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::accessibility::{
    AxNode, AxNodeId, AxProperty, AxPropertyName, AxValue, GetFullAxTreeParams,
};
use chromiumoxide::cdp::browser_protocol::dom::BackendNodeId;

use crate::Result;
use crate::aria::node::{AriaChecked, AriaChild, AriaNode, AriaPressed};
use crate::error::Error;

#[derive(Debug, Clone)]
pub(crate) struct AxRef {
    pub backend_node_id: BackendNodeId,
    pub role: String,
    pub name: String,
    pub nth: usize,
}

pub(crate) struct AxSnapshot {
    pub root: AriaNode,
    pub refs: Vec<AxRef>,
}

pub(crate) async fn extract_ax_tree(page: &Page) -> Result<AxSnapshot> {
    let resp = page
        .execute(GetFullAxTreeParams::default())
        .await
        .map_err(Error::Cdp)?;
    let nodes = resp.result.nodes.clone();

    let by_id: HashMap<&AxNodeId, &AxNode> = nodes.iter().map(|n| (&n.node_id, n)).collect();

    let root_ax = nodes
        .iter()
        .find(|n| n.parent_id.is_none())
        .or_else(|| nodes.first())
        .ok_or_else(|| Error::ToolFailed {
            tool: "snapshot",
            message: "empty AX tree".into(),
        })?;

    let mut nth_counter: HashMap<(String, String), usize> = HashMap::new();
    let mut refs: Vec<AxRef> = Vec::new();

    let mut root = AriaNode::fragment();
    if let Some(children) = collect_children(root_ax, &by_id, &mut nth_counter, &mut refs) {
        root.children = children;
    }

    Ok(AxSnapshot { root, refs })
}

fn convert_node(
    ax: &AxNode,
    by_id: &HashMap<&AxNodeId, &AxNode>,
    nth_counter: &mut HashMap<(String, String), usize>,
    refs: &mut Vec<AxRef>,
) -> Option<AriaNode> {
    let role = ax_string(&ax.role).unwrap_or_else(|| "generic".to_string());
    let name = ax_string(&ax.name).unwrap_or_default();

    if ax.ignored && role.is_empty() {
        return None;
    }
    if role == "InlineTextBox" || role == "StaticText" && name.trim().is_empty() {
        return None;
    }

    let mut node = AriaNode::new(role.clone(), name.clone());

    if let Some(props) = &ax.properties {
        apply_properties(props, &mut node);
    }
    if let Some(val) = &ax.value
        && let Some(s) = val.value.as_ref().and_then(|v| v.as_str())
        && !s.is_empty()
    {
        node.props.insert("value".into(), s.to_string());
    }

    if is_interactive(&role)
        && let Some(bid) = ax.backend_dom_node_id
    {
        let key = (role.clone(), name.clone());
        let nth = *nth_counter
            .entry(key)
            .and_modify(|n| *n += 1)
            .or_insert(0);
        node.index = Some(refs.len());
        node.box_info.visible = true;
        refs.push(AxRef {
            backend_node_id: bid,
            role,
            name,
            nth,
        });
    } else if !ax.ignored {
        node.box_info.visible = true;
    }

    if let Some(children) = collect_children(ax, by_id, nth_counter, refs) {
        node.children = children;
    }

    Some(node)
}

fn collect_children(
    ax: &AxNode,
    by_id: &HashMap<&AxNodeId, &AxNode>,
    nth_counter: &mut HashMap<(String, String), usize>,
    refs: &mut Vec<AxRef>,
) -> Option<Vec<AriaChild>> {
    let child_ids = ax.child_ids.as_ref()?;
    let mut children: Vec<AriaChild> = Vec::with_capacity(child_ids.len());
    for cid in child_ids {
        let Some(child_ax) = by_id.get(cid) else {
            continue;
        };
        let role = ax_string(&child_ax.role).unwrap_or_default();
        let name = ax_string(&child_ax.name).unwrap_or_default();
        if matches!(role.as_str(), "StaticText" | "InlineTextBox") && !name.trim().is_empty() {
            children.push(AriaChild::Text(name));
            continue;
        }
        if let Some(child) = convert_node(child_ax, by_id, nth_counter, refs) {
            children.push(AriaChild::Node(Box::new(child)));
        }
    }
    Some(children)
}

fn ax_string(v: &Option<AxValue>) -> Option<String> {
    v.as_ref()
        .and_then(|av| av.value.as_ref())
        .and_then(|val| val.as_str().map(String::from))
}

fn ax_bool(v: &serde_json::Value) -> Option<bool> {
    v.as_bool().or_else(|| {
        v.as_str().and_then(|s| match s {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    })
}

fn apply_properties(props: &[AxProperty], node: &mut AriaNode) {
    for p in props {
        let raw = match &p.value.value {
            Some(v) => v,
            None => continue,
        };
        match p.name {
            AxPropertyName::Disabled => {
                if let Some(b) = ax_bool(raw) {
                    node.disabled = Some(b);
                }
            }
            AxPropertyName::Expanded => {
                if let Some(b) = ax_bool(raw) {
                    node.expanded = Some(b);
                }
            }
            AxPropertyName::Selected => {
                if let Some(b) = ax_bool(raw) {
                    node.selected = Some(b);
                }
            }
            AxPropertyName::Focused => {
                if let Some(b) = ax_bool(raw) {
                    node.active = Some(b);
                }
            }
            AxPropertyName::Level => {
                if let Some(l) = raw.as_i64().or_else(|| raw.as_str().and_then(|s| s.parse().ok()))
                    && l >= 0
                {
                    node.level = Some(l as u32);
                }
            }
            AxPropertyName::Checked => {
                node.checked = match raw.as_str() {
                    Some("true") => Some(AriaChecked::Bool(true)),
                    Some("false") => Some(AriaChecked::Bool(false)),
                    Some("mixed") => Some(AriaChecked::Mixed("mixed".into())),
                    _ => ax_bool(raw).map(AriaChecked::Bool),
                };
            }
            AxPropertyName::Pressed => {
                node.pressed = match raw.as_str() {
                    Some("true") => Some(AriaPressed::Bool(true)),
                    Some("false") => Some(AriaPressed::Bool(false)),
                    Some("mixed") => Some(AriaPressed::Mixed("mixed".into())),
                    _ => ax_bool(raw).map(AriaPressed::Bool),
                };
            }
            AxPropertyName::Url => {
                if let Some(s) = raw.as_str()
                    && !s.is_empty()
                {
                    node.props.insert("url".into(), s.to_string());
                }
            }
            _ => {}
        }
    }
}

fn is_interactive(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "combobox"
            | "searchbox"
            | "checkbox"
            | "radio"
            | "switch"
            | "slider"
            | "spinbutton"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "tab"
            | "option"
            | "treeitem"
    )
}
