use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};
use chromiumoxide::keys::get_key_definition;

use crate::Result;
use crate::error::Error;

pub(crate) fn build_key_events(
    key: &str,
) -> Result<(DispatchKeyEventParams, DispatchKeyEventParams)> {
    let def = get_key_definition(key).ok_or_else(|| Error::ToolFailed {
        tool: "press_key",
        message: format!("unknown key: {key}"),
    })?;

    let text = def.text.or(if def.key.len() == 1 {
        Some(def.key)
    } else {
        None
    });

    let down_type = if text.is_some() {
        DispatchKeyEventType::KeyDown
    } else {
        DispatchKeyEventType::RawKeyDown
    };

    let mut down = DispatchKeyEventParams::builder()
        .r#type(down_type)
        .key(def.key)
        .code(def.code)
        .windows_virtual_key_code(def.key_code)
        .native_virtual_key_code(def.key_code);

    if let Some(t) = text {
        down = down.text(t);
    }

    let down = down.build().map_err(tool_error)?;

    let up = DispatchKeyEventParams::builder()
        .r#type(DispatchKeyEventType::KeyUp)
        .key(def.key)
        .code(def.code)
        .windows_virtual_key_code(def.key_code)
        .native_virtual_key_code(def.key_code)
        .build()
        .map_err(tool_error)?;

    Ok((down, up))
}

fn tool_error(e: String) -> Error {
    Error::ToolFailed {
        tool: "press_key",
        message: e,
    }
}
