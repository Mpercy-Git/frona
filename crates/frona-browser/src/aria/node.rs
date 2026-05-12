use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AriaNode {
    pub role: String,
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,

    #[serde(default)]
    pub children: Vec<AriaChild>,

    #[serde(default)]
    pub props: HashMap<String, String>,

    #[serde(default)]
    pub box_info: BoxInfo,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<AriaChecked>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressed: Option<AriaPressed>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AriaChild {
    Text(String),
    Node(Box<AriaNode>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AriaChecked {
    Bool(bool),
    Mixed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AriaPressed {
    Bool(bool),
    Mixed(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BoxInfo {
    #[serde(default)]
    pub visible: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl AriaNode {
    pub fn new(role: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            name: name.into(),
            index: None,
            children: Vec::new(),
            props: HashMap::new(),
            box_info: BoxInfo::default(),
            checked: None,
            disabled: None,
            expanded: None,
            level: None,
            pressed: None,
            selected: None,
            active: None,
        }
    }

    pub fn fragment() -> Self {
        Self::new("fragment", "")
    }

    pub fn has_pointer_cursor(&self) -> bool {
        self.box_info.cursor.as_deref() == Some("pointer")
    }
}

#[cfg(test)]
impl AriaNode {
    pub fn with_index(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    pub fn with_prop(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.props.insert(key.into(), value.into());
        self
    }

    pub fn with_box(mut self, visible: bool, cursor: Option<String>) -> Self {
        self.box_info = BoxInfo { visible, cursor };
        self
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = Some(AriaChecked::Bool(checked));
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    pub fn with_level(mut self, level: u32) -> Self {
        self.level = Some(level);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_pointer_cursor_distinguishes_pointer_from_default() {
        let with_pointer = AriaNode::new("button", "").with_box(true, Some("pointer".to_string()));
        assert!(with_pointer.has_pointer_cursor());

        let without_pointer =
            AriaNode::new("button", "").with_box(true, Some("default".to_string()));
        assert!(!without_pointer.has_pointer_cursor());
    }
}
