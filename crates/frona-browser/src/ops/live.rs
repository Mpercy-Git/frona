//! Live view: stream a tab's pixels out and replay a person's mouse and keyboard
//! input back in, over the same CDP connection the agent drives.
//!
//! This is what a human takeover runs on. It attaches to the agent's own browser
//! session rather than launching a second browser, so the person sees exactly the
//! page the agent got stuck on, and the cookies they leave behind (a solved
//! CAPTCHA, a completed login) are the ones the agent picks up when it resumes.

use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    InsertTextParams, MouseButton,
};
use chromiumoxide::cdp::browser_protocol::page::{
    EventScreencastFrame, ScreencastFrameAckParams, StartScreencastFormat, StartScreencastParams,
    StopScreencastParams,
};
use chromiumoxide::keys::get_key_definition;
use chromiumoxide::listeners::EventStream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::url::normalize_url;

/// JPEG keeps a frame of a typical page in the tens of kilobytes; PNG would be
/// several times that for no visible gain at screencast sizes.
const FRAME_QUALITY: i64 = 70;
/// Upper bound on the encoded frame. Chrome scales down to fit, and the viewer
/// maps input through the frame's CSS-pixel metadata, so the cap only trades
/// sharpness for bandwidth.
const FRAME_MAX_WIDTH: i64 = 1600;
const FRAME_MAX_HEIGHT: i64 = 1600;

/// Modifier bits as CDP's `Input` domain defines them.
const MODIFIER_CTRL: i64 = 2;
const MODIFIER_META: i64 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveTab {
    /// The CDP target id. Stable for the tab's lifetime, unlike its index.
    pub id: String,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct LiveFrame {
    /// Base64-encoded JPEG, exactly as Chrome sent it.
    pub data: String,
    /// The page's viewport in CSS pixels. Input coordinates are in this space,
    /// whatever size the encoded image came out at.
    pub width: f64,
    pub height: f64,
    /// Handed back to [`Screencast::ack`] once the frame has been delivered.
    pub ack: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Pressed,
    Released,
    Moved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Down,
    Up,
}

/// One thing the person did in the viewer. Coordinates are CSS pixels in the
/// page's viewport, which the viewer derives from the frame's `width`/`height`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveCommand {
    Mouse {
        action: MouseAction,
        x: f64,
        y: f64,
        /// `left`, `middle`, `right`, `back`, `forward` or `none`.
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        buttons: i64,
        #[serde(default)]
        click_count: i64,
        #[serde(default)]
        modifiers: i64,
    },
    Wheel {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        #[serde(default)]
        modifiers: i64,
    },
    Key {
        action: KeyAction,
        /// `KeyboardEvent.key`, e.g. `a`, `A`, `Enter`, `ArrowLeft`.
        key: String,
        /// `KeyboardEvent.code`, e.g. `KeyA`, `Enter`.
        #[serde(default)]
        code: String,
        #[serde(default)]
        key_code: i64,
        #[serde(default)]
        modifiers: i64,
    },
    /// Text that arrives whole rather than as key presses: a paste, an IME
    /// composition, or a phone's on-screen keyboard.
    Text {
        text: String,
    },
    Navigate {
        url: String,
    },
    Back,
    Forward,
    Reload,
}

/// A running screencast of one tab. Stop it with [`Screencast::stop`]; dropping
/// it leaves Chrome encoding frames for nobody until the tab closes.
pub struct Screencast {
    page: Page,
    frames: EventStream<EventScreencastFrame>,
}

impl BrowserConnection {
    /// Every open tab, in the browser's own order.
    pub async fn live_tabs(&self) -> Result<Vec<LiveTab>> {
        let pages = self.pages().await?;
        let mut out = Vec::with_capacity(pages.len());
        for page in &pages {
            out.push(LiveTab {
                id: page.target_id().as_ref().to_string(),
                url: page.url().await.ok().flatten().unwrap_or_default(),
                title: page.get_title().await.ok().flatten().unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// The tab the agent would act on next: the same choice its own tools make.
    pub async fn live_active_tab(&self) -> Result<String> {
        Ok(self.active_page().await?.target_id().as_ref().to_string())
    }

    /// Start streaming `tab_id`, or the agent's active tab when `None`. The tab
    /// is brought to the front, so it is also where the agent carries on.
    pub async fn start_screencast(&self, tab_id: Option<&str>) -> Result<Screencast> {
        let page = match tab_id {
            Some(id) => self
                .pages()
                .await?
                .into_iter()
                .find(|p| p.target_id().as_ref() == id)
                .ok_or(Error::NoActivePage)?,
            None => self.active_page().await?,
        };
        page.bring_to_front().await.map_err(Error::Cdp)?;

        // Listen before starting so the first frame can't slip past.
        let frames = page
            .event_listener::<EventScreencastFrame>()
            .await
            .map_err(Error::Cdp)?;
        let params = StartScreencastParams::builder()
            .format(StartScreencastFormat::Jpeg)
            .quality(FRAME_QUALITY)
            .max_width(FRAME_MAX_WIDTH)
            .max_height(FRAME_MAX_HEIGHT)
            .build();
        page.execute(params).await.map_err(Error::Cdp)?;

        Ok(Screencast { page, frames })
    }
}

impl Screencast {
    pub fn tab_id(&self) -> &str {
        self.page.target_id().as_ref()
    }

    /// The next frame, or `None` once the tab has gone away. Cancel-safe.
    pub async fn next_frame(&mut self) -> Option<LiveFrame> {
        let frame = self.frames.next().await?;
        let data: &str = frame.data.as_ref();
        Some(LiveFrame {
            data: data.to_string(),
            width: frame.metadata.device_width,
            height: frame.metadata.device_height,
            ack: frame.session_id,
        })
    }

    /// Ask for the next frame. Chrome holds further frames until this arrives,
    /// which is what keeps a slow viewer from being buried in stale ones.
    pub async fn ack(&self, ack: i64) -> Result<()> {
        self.page
            .execute(ScreencastFrameAckParams::new(ack))
            .await
            .map_err(Error::Cdp)?;
        Ok(())
    }

    pub async fn stop(self) -> Result<()> {
        self.page
            .execute(StopScreencastParams::default())
            .await
            .map_err(Error::Cdp)?;
        Ok(())
    }

    pub async fn apply(&self, command: LiveCommand) -> Result<()> {
        match command {
            LiveCommand::Mouse {
                action,
                x,
                y,
                button,
                buttons,
                click_count,
                modifiers,
            } => {
                let kind = match action {
                    MouseAction::Pressed => DispatchMouseEventType::MousePressed,
                    MouseAction::Released => DispatchMouseEventType::MouseReleased,
                    MouseAction::Moved => DispatchMouseEventType::MouseMoved,
                };
                let button = button
                    .as_deref()
                    .and_then(|b| b.parse::<MouseButton>().ok())
                    .unwrap_or(MouseButton::None);
                let params = DispatchMouseEventParams::builder()
                    .r#type(kind)
                    .x(x)
                    .y(y)
                    .button(button)
                    .buttons(buttons)
                    .click_count(click_count)
                    .modifiers(modifiers)
                    .build()
                    .map_err(input_error)?;
                self.page.execute(params).await.map_err(Error::Cdp)?;
            }
            LiveCommand::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                modifiers,
            } => {
                let params = DispatchMouseEventParams::builder()
                    .r#type(DispatchMouseEventType::MouseWheel)
                    .x(x)
                    .y(y)
                    .delta_x(delta_x)
                    .delta_y(delta_y)
                    .modifiers(modifiers)
                    .build()
                    .map_err(input_error)?;
                self.page.execute(params).await.map_err(Error::Cdp)?;
            }
            LiveCommand::Key {
                action,
                key,
                code,
                key_code,
                modifiers,
            } => {
                self.page
                    .execute(key_event(action, &key, &code, key_code, modifiers)?)
                    .await
                    .map_err(Error::Cdp)?;
            }
            LiveCommand::Text { text } => {
                if !text.is_empty() {
                    self.page
                        .execute(InsertTextParams::new(text))
                        .await
                        .map_err(Error::Cdp)?;
                }
            }
            LiveCommand::Navigate { url } => {
                let url = live_navigation_url(&url)?;
                self.page.goto(url).await.map_err(Error::Cdp)?;
            }
            LiveCommand::Back => {
                self.page
                    .evaluate("window.history.back()")
                    .await
                    .map_err(Error::Cdp)?;
            }
            LiveCommand::Forward => {
                self.page
                    .evaluate("window.history.forward()")
                    .await
                    .map_err(Error::Cdp)?;
            }
            LiveCommand::Reload => {
                self.page.reload().await.map_err(Error::Cdp)?;
            }
        }
        Ok(())
    }
}

/// Build the CDP key event for a browser `KeyboardEvent`.
///
/// A key that produces a character must carry it as `text`, or Chrome fires the
/// keydown but never types anything. `Enter` is the trap: its `key` is the word
/// "Enter", but forms only submit when the event carries `\r`, which the key
/// table supplies. With Ctrl or Meta held the key is a shortcut, not typing, so
/// it goes without text (Ctrl+A selects instead of inserting an "a").
fn key_event(
    action: KeyAction,
    key: &str,
    code: &str,
    key_code: i64,
    modifiers: i64,
) -> Result<DispatchKeyEventParams> {
    let def = get_key_definition(key);
    let code = if code.is_empty() {
        def.map(|d| d.code).unwrap_or_default()
    } else {
        code
    };
    let key_code = if key_code == 0 {
        def.map(|d| d.key_code).unwrap_or_default()
    } else {
        key_code
    };

    let shortcut = modifiers & (MODIFIER_CTRL | MODIFIER_META) != 0;
    let text = if shortcut {
        None
    } else if key.chars().count() == 1 {
        Some(key.to_string())
    } else {
        def.and_then(|d| d.text).map(str::to_string)
    };

    let kind = match (action, &text) {
        (KeyAction::Up, _) => DispatchKeyEventType::KeyUp,
        (KeyAction::Down, Some(_)) => DispatchKeyEventType::KeyDown,
        (KeyAction::Down, None) => DispatchKeyEventType::RawKeyDown,
    };

    let mut builder = DispatchKeyEventParams::builder()
        .r#type(kind)
        .key(key)
        .code(code)
        .windows_virtual_key_code(key_code)
        .native_virtual_key_code(key_code)
        .modifiers(modifiers);
    if action == KeyAction::Down
        && let Some(text) = text
    {
        builder = builder.text(text.clone()).unmodified_text(text);
    }
    builder.build().map_err(input_error)
}

/// The live view is for getting a person past a login or a CAPTCHA, so its
/// address bar only goes to web pages: a `file://` or `chrome://` URL typed here
/// would open the browser container itself to whoever holds the viewer.
fn live_navigation_url(url: &str) -> Result<String> {
    let url = normalize_url(url);
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower == "about:blank" {
        Ok(url)
    } else {
        Err(Error::ToolFailed {
            tool: "live_view",
            message: format!("only http(s) addresses can be opened here, not {url:?}"),
        })
    }
}

fn input_error(message: String) -> Error {
    Error::ToolFailed {
        tool: "live_view",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_key_carries_its_character() {
        let ev = key_event(KeyAction::Down, "a", "KeyA", 65, 0).unwrap();
        assert_eq!(ev.r#type, DispatchKeyEventType::KeyDown);
        assert_eq!(ev.text.as_deref(), Some("a"));
    }

    #[test]
    fn enter_carries_carriage_return_so_forms_submit() {
        let ev = key_event(KeyAction::Down, "Enter", "Enter", 13, 0).unwrap();
        assert_eq!(ev.r#type, DispatchKeyEventType::KeyDown);
        assert_eq!(ev.text.as_deref(), Some("\r"));
    }

    #[test]
    fn non_printing_key_is_a_raw_keydown_without_text() {
        let ev = key_event(KeyAction::Down, "ArrowLeft", "ArrowLeft", 37, 0).unwrap();
        assert_eq!(ev.r#type, DispatchKeyEventType::RawKeyDown);
        assert_eq!(ev.text, None);
    }

    #[test]
    fn ctrl_shortcut_does_not_type_its_letter() {
        let ev = key_event(KeyAction::Down, "a", "KeyA", 65, MODIFIER_CTRL).unwrap();
        assert_eq!(ev.r#type, DispatchKeyEventType::RawKeyDown);
        assert_eq!(ev.text, None);
    }

    #[test]
    fn key_up_never_carries_text() {
        let ev = key_event(KeyAction::Up, "a", "KeyA", 65, 0).unwrap();
        assert_eq!(ev.r#type, DispatchKeyEventType::KeyUp);
        assert_eq!(ev.text, None);
    }

    #[test]
    fn missing_code_is_filled_from_the_key_table() {
        let ev = key_event(KeyAction::Down, "Backspace", "", 0, 0).unwrap();
        assert_eq!(ev.code.as_deref(), Some("Backspace"));
        assert_eq!(ev.windows_virtual_key_code, Some(8));
    }

    #[test]
    fn navigation_is_limited_to_web_pages() {
        assert_eq!(
            live_navigation_url("example.com").unwrap(),
            "https://example.com"
        );
        assert!(live_navigation_url("https://example.com/login").is_ok());
        assert!(live_navigation_url("about:blank").is_ok());
        assert!(live_navigation_url("file:///etc/passwd").is_err());
        assert!(live_navigation_url("chrome://settings").is_err());
    }

    #[test]
    fn commands_parse_from_viewer_json() {
        let cmd: LiveCommand = serde_json::from_str(
            r#"{"type":"mouse","action":"pressed","x":10.5,"y":20,"button":"left","buttons":1,"click_count":1}"#,
        )
        .unwrap();
        assert!(matches!(
            cmd,
            LiveCommand::Mouse {
                action: MouseAction::Pressed,
                click_count: 1,
                ..
            }
        ));
        let cmd: LiveCommand = serde_json::from_str(r#"{"type":"reload"}"#).unwrap();
        assert_eq!(cmd, LiveCommand::Reload);
    }
}
