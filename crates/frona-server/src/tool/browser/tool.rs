use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::core::error::AppError;
use crate::credential::vault::service::VaultService;
use crate::tool::{AgentTool, ImageData, InferenceContext, ToolDefinition, ToolOutput};

use super::session::{BrowserSessionManager, run_with_reconnect};
use frona_browser::{ElementTarget, ExtractFormat};

pub struct BrowserTool {
    session_manager: Arc<BrowserSessionManager>,
    vault_service: VaultService,
}

impl BrowserTool {
    pub fn new(session_manager: Arc<BrowserSessionManager>, vault_service: VaultService) -> Self {
        Self {
            session_manager,
            vault_service,
        }
    }
}

fn default_true() -> bool {
    true
}

/// How much post-action page state one action result may carry before it is cut off.
/// A compact ARIA diff after a click is usually a few lines; a full compact tree for a
/// dense page is the thing worth bounding.
const PAGE_STATE_MAX_CHARS: usize = 8000;

/// How long to let the page settle before reading it back. A click that starts a
/// navigation or opens a menu returns before the DOM catches up, and the agent's own
/// follow-up `browser_snapshot` used to provide this delay incidentally — a whole model
/// round-trip of it. A fraction of a second buys most of that back for a fraction of
/// the cost; anything slower is what `browser_wait` is for.
const PAGE_SETTLE: Duration = Duration::from_millis(400);

/// What the agent should see of the page an action just changed.
#[derive(Clone, Copy)]
enum PageState {
    /// A unified diff against the page as it was before the action — a click or a
    /// keystroke usually moves a handful of lines, and the diff is what the agent
    /// would have gone looking for next.
    Diff,
    /// The whole compact tree, for actions that replace the page outright
    /// (navigation, tab switches) where a diff would be the entire page anyway.
    Full,
}

/// Render the page as it stands after an action.
///
/// Every state-changing browser action used to answer with a bare acknowledgement
/// ("clicked"), which said nothing about the page the agent had just changed — so each
/// one was reliably followed by a `browser_snapshot` call, and a browser task cost
/// twice the tool turns it needed. The action now carries that state itself.
///
/// Best-effort on purpose: the action already happened, so a snapshot that fails must
/// not turn a successful click into an error. The agent still has `browser_snapshot`.
async fn page_state_after(
    mgr: &BrowserSessionManager,
    session_key: &crate::core::Handle,
    provider: &str,
    mode: PageState,
) -> String {
    tokio::time::sleep(PAGE_SETTLE).await;
    let incremental = matches!(mode, PageState::Diff);
    let snap = run_with_reconnect(mgr, session_key, provider, |c| async move {
        c.snapshot(incremental, true).await
    })
    .await;
    let snap = match snap {
        Ok(snap) => snap,
        Err(e) => {
            tracing::warn!(error = %e, "browser: could not snapshot the page after an action");
            return String::new();
        }
    };

    render_page_state(snap.diffed, snap.tree, snap.interactive_count)
}

/// The suffix an action's result carries. Split out from the snapshot call so the
/// bounding and the "you already have this" wording are testable without a browser.
///
/// `diffed` comes from the snapshot rather than from the requested mode: a diff was
/// asked for, but the first look at a page has nothing to diff against and answers with
/// the whole tree. Labelling that as "changes" would read as "everything just changed".
fn render_page_state(diffed: bool, tree: String, interactive_count: usize) -> String {
    let label = if diffed {
        "page changes (compact ARIA diff)"
    } else {
        "page now (compact ARIA)"
    };
    let mut tree = if tree.trim().is_empty() {
        if diffed {
            "(nothing changed on the page)".to_string()
        } else {
            "(no interactive content on the page)".to_string()
        }
    } else {
        tree
    };
    if let Some(cut) = tree
        .char_indices()
        .nth(PAGE_STATE_MAX_CHARS)
        .map(|(i, _)| i)
    {
        tree.truncate(cut);
        tree.push_str("\n[truncated — call browser_snapshot for the full tree]");
    }
    format!(
        "\n\n--- {label}, {interactive_count} interactive elements. This is the post-action state: don't call browser_snapshot just to see it. ---\n{tree}"
    )
}

fn element_target(
    selector: Option<&str>,
    index: Option<usize>,
) -> Result<ElementTarget<'_>, AppError> {
    match (selector, index) {
        (Some(s), None) => Ok(ElementTarget::Selector(s)),
        (None, Some(i)) => Ok(ElementTarget::Index(i)),
        (Some(_), Some(_)) => Err(AppError::Validation(
            "specify either selector or index, not both".into(),
        )),
        (None, None) => Err(AppError::Validation(
            "must specify selector or index".into(),
        )),
    }
}

#[async_trait]
impl AgentTool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_navigate".to_string(),
                description: "Navigate to a URL in the browser. Returns the loaded page as a compact ARIA tree with indexed interactive elements — no follow-up browser_snapshot needed.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL to navigate to" },
                        "wait_for_load": { "type": "boolean", "description": "Wait for navigation to complete", "default": true }
                    },
                    "required": ["url"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_go_back".to_string(),
                description: "Navigate back in browser history. Returns the resulting page as a compact ARIA tree.".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_go_forward".to_string(),
                description: "Navigate forward in browser history. Returns the resulting page as a compact ARIA tree.".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_close".to_string(),
                description: "Close the browser when the task is complete.".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_extract".to_string(),
                description: "Extract text or HTML content from the current page or a specific element.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type":"string","description":"CSS selector to extract from (defaults to body)"},
                        "format": {"type":"string","enum":["text","html"],"description":"Output format","default":"text"}
                    }
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_read_links".to_string(),
                description: "Get all links on the current page with their text and URLs.".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_get_markdown".to_string(),
                description: "Get the markdown content of the current page with pagination support.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "page": {"type":"integer","minimum":1,"description":"Page number to extract (1-based)","default":1},
                        "page_size": {"type":"integer","minimum":1,"description":"Maximum characters per page","default":100000}
                    }
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_snapshot".to_string(),
                description: "Get an ARIA snapshot of the page with indexed interactive elements. Actions already return the page they changed, so reach for this only when you need the FULL tree (e.g. the diff an action returned was truncated, or the page changed by itself).".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "incremental": {"type":"boolean","description":"Return unified diff against the previous snapshot instead of full tree","default":false},
                        "compact": {"type":"boolean","description":"Strip non-actionable lines from the output to save tokens","default":false}
                    }
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_screenshot".to_string(),
                description: "Capture a screenshot of the current page.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type":"string","description":"Path to save the screenshot"},
                        "full_page": {"type":"boolean","description":"Capture full page instead of viewport","default":false}
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_evaluate".to_string(),
                description: "Execute JavaScript code in the browser context.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {"type":"string","description":"JavaScript code to execute"},
                        "await_promise": {"type":"boolean","description":"Wait for promise resolution","default":false}
                    },
                    "required": ["code"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_click".to_string(),
                description: "Click on a DOM element by CSS selector or snapshot index. Returns what the click changed on the page — don't follow it with browser_snapshot.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type":"string","description":"CSS selector of the element to click"},
                        "index": {"type":"integer","minimum":0,"description":"Snapshot index of the element to click"}
                    }
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_hover".to_string(),
                description: "Hover over a DOM element by CSS selector or snapshot index. Returns what the hover changed on the page.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type":"string","description":"CSS selector of the element to hover"},
                        "index": {"type":"integer","minimum":0,"description":"Snapshot index of the element to hover"}
                    }
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_select".to_string(),
                description: "Select an option in a dropdown element. Returns what the selection changed on the page.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type":"string","description":"CSS selector of the dropdown"},
                        "index": {"type":"integer","minimum":0,"description":"Snapshot index of the dropdown"},
                        "value": {"type":"string","description":"Value to select in the dropdown"}
                    },
                    "required": ["value"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_input_fill".to_string(),
                description: "Type text into an input element by CSS selector or snapshot index. Returns what typing changed on the page — don't follow it with browser_snapshot.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type":"string","description":"CSS selector of the input element"},
                        "index": {"type":"integer","minimum":0,"description":"Snapshot index of the input element"},
                        "text": {"type":"string","description":"Text to type into the element"},
                        "clear": {"type":"boolean","description":"Clear existing content first","default":false}
                    },
                    "required": ["text"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_press_key".to_string(),
                description: "Press a keyboard key (e.g. Enter, Tab, Escape, ArrowDown). Returns what the keypress changed on the page.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"key": {"type":"string","description":"Name of the key to press"}},
                    "required": ["key"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_scroll".to_string(),
                description: "Scroll the page by a pixel amount. Positive = down, negative = up. Omit to scroll to bottom. Returns what scrolling brought into the tree.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"amount": {"type":"integer","description":"Pixels to scroll (positive=down, negative=up). Omit to scroll to bottom."}}
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_wait".to_string(),
                description: "Wait for a DOM element matching a CSS selector to appear. Returns what changed on the page while waiting.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type":"string","description":"CSS selector to wait for"},
                        "timeout_ms": {"type":"integer","minimum":0,"description":"Timeout in milliseconds","default":30000}
                    },
                    "required": ["selector"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_new_tab".to_string(),
                description: "Open a new tab and navigate to the specified URL. Returns the new tab's page as a compact ARIA tree.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"url": {"type":"string","description":"URL to open in the new tab"}},
                    "required": ["url"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_tab_list".to_string(),
                description: "List all open browser tabs with their titles and URLs.".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_switch_tab".to_string(),
                description: "Switch to a specific tab by index. Returns that tab's page as a compact ARIA tree.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"index": {"type":"integer","minimum":0,"description":"Tab index to switch to (0-based)"}},
                    "required": ["index"]
                }),
            },
            ToolDefinition {
                provider_id: "browser".to_string(),
                id: "browser_close_tab".to_string(),
                description: "Close the current active tab. Returns the page that becomes active as a compact ARIA tree.".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
        ]
    }

    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let session_key = &ctx.user.handle;
        let provider = self
            .vault_service
            .list_credentials(&ctx.user.id)
            .await
            .ok()
            .and_then(|creds| creds.into_iter().next())
            .map(|c| c.provider)
            .unwrap_or_else(|| "default".to_string());
        let mgr = self.session_manager.as_ref();

        match tool_name {
            "browser_navigate" => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    #[serde(default = "default_true")]
                    wait_for_load: bool,
                }
                let p: P = serde_json::from_value(arguments)?;
                let url = p.url.as_str();
                let wait = p.wait_for_load;
                let info = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.navigate(url, wait).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Full).await;
                Ok(ToolOutput::text(format!(
                    "{}{state}",
                    serde_json::to_string(&info)?
                )))
            }
            "browser_go_back" => {
                run_with_reconnect(
                    mgr,
                    session_key,
                    &provider,
                    |c| async move { c.go_back().await },
                )
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Full).await;
                Ok(ToolOutput::text(format!("navigated back{state}")))
            }
            "browser_go_forward" => {
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.go_forward().await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Full).await;
                Ok(ToolOutput::text(format!("navigated forward{state}")))
            }
            "browser_close" => {
                mgr.close_session(session_key, &provider).await?;
                Ok(ToolOutput::text("Browser closed."))
            }
            "browser_extract" => {
                #[derive(Deserialize)]
                struct P {
                    selector: Option<String>,
                    #[serde(default)]
                    format: Option<String>,
                }
                let p: P = serde_json::from_value(arguments)?;
                let format = match p.format.as_deref() {
                    Some("html") => ExtractFormat::Html,
                    _ => ExtractFormat::Text,
                };
                let selector = p.selector.as_deref();
                let content = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.extract(selector, format).await
                })
                .await?;
                Ok(ToolOutput::text(content))
            }
            "browser_read_links" => {
                let links = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.read_links().await
                })
                .await?;
                Ok(ToolOutput::text(serde_json::to_string(&links)?))
            }
            "browser_get_markdown" => {
                #[derive(Deserialize)]
                struct P {
                    #[serde(default = "page_default")]
                    page: usize,
                    #[serde(default = "page_size_default")]
                    page_size: usize,
                }
                fn page_default() -> usize {
                    1
                }
                fn page_size_default() -> usize {
                    100_000
                }
                let p: P = serde_json::from_value(arguments).unwrap_or(P {
                    page: 1,
                    page_size: 100_000,
                });
                let md = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.get_markdown(p.page, p.page_size).await
                })
                .await?;
                Ok(ToolOutput::text(serde_json::to_string(&md)?))
            }
            "browser_snapshot" => {
                #[derive(Deserialize)]
                struct P {
                    #[serde(default)]
                    incremental: bool,
                    #[serde(default)]
                    compact: bool,
                }
                let p: P = serde_json::from_value(arguments).unwrap_or(P {
                    incremental: false,
                    compact: false,
                });
                let snap = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.snapshot(p.incremental, p.compact).await
                })
                .await?;
                Ok(ToolOutput::text(serde_json::to_string(&snap)?))
            }
            "browser_screenshot" => {
                #[derive(Deserialize)]
                struct P {
                    path: String,
                    #[serde(default)]
                    full_page: bool,
                }
                let p: P = serde_json::from_value(arguments)?;
                let path = Path::new(&p.path);
                let full_page = p.full_page;
                let result = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.screenshot(path, full_page).await
                })
                .await?;
                let text = serde_json::to_string(&result)?;
                if let Ok(bytes) = std::fs::read(&p.path) {
                    return Ok(ToolOutput::mixed(
                        text,
                        vec![ImageData {
                            bytes,
                            media_type: "image/png".into(),
                        }],
                    ));
                }
                Ok(ToolOutput::text(text))
            }
            "browser_evaluate" => {
                #[derive(Deserialize)]
                struct P {
                    code: String,
                    #[serde(default)]
                    await_promise: bool,
                }
                let p: P = serde_json::from_value(arguments)?;
                let code = p.code.as_str();
                let await_promise = p.await_promise;
                let value = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.evaluate(code, await_promise).await
                })
                .await?;
                Ok(ToolOutput::text(serde_json::to_string(&value)?))
            }
            "browser_click" => {
                #[derive(Deserialize)]
                struct P {
                    selector: Option<String>,
                    index: Option<usize>,
                }
                let p: P = serde_json::from_value(arguments)?;
                let target = element_target(p.selector.as_deref(), p.index)?;
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.click(target).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("clicked{state}")))
            }
            "browser_hover" => {
                #[derive(Deserialize)]
                struct P {
                    selector: Option<String>,
                    index: Option<usize>,
                }
                let p: P = serde_json::from_value(arguments)?;
                let target = element_target(p.selector.as_deref(), p.index)?;
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.hover(target).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("hovered{state}")))
            }
            "browser_select" => {
                #[derive(Deserialize)]
                struct P {
                    selector: Option<String>,
                    index: Option<usize>,
                    value: String,
                }
                let p: P = serde_json::from_value(arguments)?;
                let target = element_target(p.selector.as_deref(), p.index)?;
                let value = p.value.as_str();
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.select(target, value).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("selected{state}")))
            }
            "browser_input_fill" => {
                #[derive(Deserialize)]
                struct P {
                    selector: Option<String>,
                    index: Option<usize>,
                    text: String,
                    #[serde(default)]
                    clear: bool,
                }
                let p: P = serde_json::from_value(arguments)?;
                let target = element_target(p.selector.as_deref(), p.index)?;
                let text = p.text.as_str();
                let clear = p.clear;
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.input_fill(target, text, clear).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("input filled{state}")))
            }
            "browser_press_key" => {
                #[derive(Deserialize)]
                struct P {
                    key: String,
                }
                let p: P = serde_json::from_value(arguments)?;
                let key = p.key.as_str();
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.press_key(key).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("pressed {}{state}", p.key)))
            }
            "browser_scroll" => {
                #[derive(Deserialize)]
                struct P {
                    amount: Option<i64>,
                }
                let p: P = serde_json::from_value(arguments).unwrap_or(P { amount: None });
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.scroll(p.amount).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("scrolled{state}")))
            }
            "browser_wait" => {
                #[derive(Deserialize)]
                struct P {
                    selector: String,
                    #[serde(default = "wait_default")]
                    timeout_ms: u64,
                }
                fn wait_default() -> u64 {
                    30000
                }
                let p: P = serde_json::from_value(arguments)?;
                let timeout = Duration::from_millis(p.timeout_ms);
                let selector = p.selector.as_str();
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.wait_for_selector(selector, timeout).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Diff).await;
                Ok(ToolOutput::text(format!("found {}{state}", p.selector)))
            }
            "browser_new_tab" => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                }
                let p: P = serde_json::from_value(arguments)?;
                let url = p.url.as_str();
                let info = run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.new_tab(url).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Full).await;
                Ok(ToolOutput::text(format!(
                    "{}{state}",
                    serde_json::to_string(&info)?
                )))
            }
            "browser_tab_list" => {
                let tabs =
                    run_with_reconnect(
                        mgr,
                        session_key,
                        &provider,
                        |c| async move { c.tabs().await },
                    )
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string(&tabs)?))
            }
            "browser_switch_tab" => {
                #[derive(Deserialize)]
                struct P {
                    index: usize,
                }
                let p: P = serde_json::from_value(arguments)?;
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.switch_tab(p.index).await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Full).await;
                Ok(ToolOutput::text(format!(
                    "switched to tab {}{state}",
                    p.index
                )))
            }
            "browser_close_tab" => {
                run_with_reconnect(mgr, session_key, &provider, |c| async move {
                    c.close_active_tab().await
                })
                .await?;
                let state = page_state_after(mgr, session_key, &provider, PageState::Full).await;
                Ok(ToolOutput::text(format!("tab closed{state}")))
            }
            other => Err(AppError::Tool(format!("Unknown browser sub-tool: {other}"))),
        }
    }

    async fn cleanup(&self) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the suffix: an action answers with the page it changed, so the
    /// agent has no reason to spend a second tool call on `browser_snapshot`.
    #[test]
    fn page_state_carries_the_tree_and_says_not_to_snapshot() {
        let out = render_page_state(true, "+ button \"Save\" [index=3]".into(), 12);
        assert!(out.contains("+ button \"Save\" [index=3]"));
        assert!(out.contains("12 interactive elements"));
        assert!(out.contains("don't call browser_snapshot"));
    }

    #[test]
    fn an_action_that_changed_nothing_says_so_rather_than_trailing_blank() {
        let out = render_page_state(true, "   \n".into(), 4);
        assert!(out.contains("(nothing changed on the page)"));
    }

    /// An action asks for a diff, but the first look at a page has nothing to diff
    /// against - that is the whole tree, and saying "changes" would misread as "all of
    /// it just changed".
    #[test]
    fn a_first_look_is_labelled_as_the_page_not_as_changes() {
        let out = render_page_state(false, "- button \"Save\" [index=3]".into(), 1);
        assert!(out.contains("page now (compact ARIA)"), "{out}");
        assert!(!out.contains("diff"), "{out}");
    }

    #[test]
    fn an_oversized_tree_is_bounded_and_points_at_the_full_snapshot() {
        let out = render_page_state(false, "x".repeat(PAGE_STATE_MAX_CHARS * 2), 1);
        assert!(out.len() < PAGE_STATE_MAX_CHARS * 2);
        assert!(out.ends_with("[truncated — call browser_snapshot for the full tree]"));
    }
}
