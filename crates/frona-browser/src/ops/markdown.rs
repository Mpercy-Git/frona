use serde::Deserialize;

use crate::Result;
use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::markdown::convert_html_to_markdown;
use crate::types::MarkdownPage;

const READABILITY: &str = include_str!("../js/readability.min.js");
const CONVERT_JS: &str = include_str!("../js/convert_to_markdown.js");

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ExtractionResult {
    title: String,
    content: String,
    #[serde(default)]
    readability_failed: bool,
    #[serde(default)]
    error: Option<String>,
}

impl BrowserConnection {
    pub async fn get_markdown(&self, page: usize, page_size: usize) -> Result<MarkdownPage> {
        let page_size = page_size.max(1);
        let target_page = page.max(1);

        let p = self.active_page().await?;
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        let js = format!(
            "var READABILITY_SCRIPT = {};\n{}",
            serde_json::to_string(READABILITY)?,
            CONVERT_JS
        );
        let result = p.evaluate(js).await.map_err(Error::Cdp)?;
        let raw = result.value().cloned().ok_or_else(|| Error::ToolFailed {
            tool: "get_markdown",
            message: "no value from convert_to_markdown.js".into(),
        })?;

        let extraction: ExtractionResult = match raw {
            serde_json::Value::String(s) => {
                serde_json::from_str(&s).map_err(|e| Error::ToolFailed {
                    tool: "get_markdown",
                    message: format!("parse: {e}"),
                })?
            }
            other => serde_json::from_value(other).map_err(|e| Error::ToolFailed {
                tool: "get_markdown",
                message: format!("decode: {e}"),
            })?,
        };

        if extraction.readability_failed {
            return Err(Error::ToolFailed {
                tool: "get_markdown",
                message: extraction
                    .error
                    .unwrap_or_else(|| "Readability extraction failed".into()),
            });
        }

        let full = convert_html_to_markdown(&extraction.content);
        let total_chars = full.len();
        let page_count = if full.is_empty() {
            1
        } else {
            full.len().div_ceil(page_size)
        };
        let current_page = target_page.min(page_count.max(1));
        let start = (current_page - 1) * page_size;
        let end = (start + page_size).min(full.len());
        let mut content = if start < full.len() {
            full[start..end].to_string()
        } else {
            String::new()
        };

        if current_page == 1 && !extraction.title.is_empty() {
            content = format!("# {}\n\n{}", extraction.title, content);
        }

        if page_count > 1 {
            let suffix = if current_page < page_count {
                format!(
                    "\n\n---\n\n*Page {current_page} of {page_count}. There are {} more page(s).*\n",
                    page_count - current_page
                )
            } else {
                format!("\n\n---\n\n*Page {current_page} of {page_count}. This is the last page.*\n")
            };
            content.push_str(&suffix);
        }

        Ok(MarkdownPage {
            page: current_page,
            page_count,
            content,
            total_chars,
        })
    }
}
