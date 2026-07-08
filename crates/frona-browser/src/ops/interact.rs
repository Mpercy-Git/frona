use chromiumoxide::cdp::browser_protocol::dom::{BackendNodeId, ResolveNodeParams};
use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;

use crate::Result;
use crate::aria::axtree::AxRef;
use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::types::ElementTarget;

impl BrowserConnection {
    pub async fn click(&self, target: ElementTarget<'_>) -> Result<()> {
        let selector = self.resolve_to_selector(target).await?;
        let page = self.active_page().await?;
        let el = page.find_element(selector).await.map_err(Error::Cdp)?;
        el.scroll_into_view().await.map_err(Error::Cdp)?;
        el.click().await.map_err(Error::Cdp)?;
        Ok(())
    }

    pub async fn hover(&self, target: ElementTarget<'_>) -> Result<()> {
        let selector = self.resolve_to_selector(target).await?;
        let page = self.active_page().await?;
        let el = page.find_element(selector).await.map_err(Error::Cdp)?;
        el.scroll_into_view().await.map_err(Error::Cdp)?;
        el.hover().await.map_err(Error::Cdp)?;
        Ok(())
    }

    pub async fn select(&self, target: ElementTarget<'_>, value: &str) -> Result<()> {
        let selector = self.resolve_to_selector(target).await?;
        let page = self.active_page().await?;
        let el = page.find_element(selector).await.map_err(Error::Cdp)?;
        let escaped = serde_json::to_string(value).map_err(Error::Json)?;
        let fn_decl = format!(
            "function() {{\
                const v = {escaped};\
                for (const opt of this.options) {{\
                    if (opt.value === v || opt.textContent.trim() === v) {{\
                        opt.selected = true;\
                        this.value = opt.value;\
                        this.dispatchEvent(new Event('input', {{bubbles:true}}));\
                        this.dispatchEvent(new Event('change', {{bubbles:true}}));\
                        return true;\
                    }}\
                }}\
                return false;\
            }}"
        );
        let r = el.call_js_fn(fn_decl, false).await.map_err(Error::Cdp)?;
        let ok = r.result.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            return Err(Error::ToolFailed {
                tool: "select",
                message: format!("no <option> matched value/text {value:?}"),
            });
        }
        Ok(())
    }

    pub async fn input_fill(
        &self,
        target: ElementTarget<'_>,
        text: &str,
        clear: bool,
    ) -> Result<()> {
        let selector = self.resolve_to_selector(target).await?;
        let page = self.active_page().await?;
        let el = page.find_element(selector).await.map_err(Error::Cdp)?;
        el.click().await.map_err(Error::Cdp)?;
        if clear {
            el.call_js_fn(
                "function() { this.value = ''; \
                    this.dispatchEvent(new Event('input', { bubbles: true })); \
                    this.dispatchEvent(new Event('change', { bubbles: true })); }",
                false,
            )
            .await
            .map_err(Error::Cdp)?;
        }
        el.type_str(text).await.map_err(Error::Cdp)?;
        Ok(())
    }

    async fn resolve_to_selector(&self, target: ElementTarget<'_>) -> Result<String> {
        match target {
            ElementTarget::Selector(s) => Ok(s.to_string()),
            ElementTarget::Index(i) => {
                let r = self.lookup_snapshot_ref(i)?;
                let page = self.active_page().await?;
                if let Ok(sel) = try_tag(&page, &r).await {
                    return Ok(sel);
                }
                // Cached backend_node_id is stale (DOM mutated). Re-walk the AX
                // tree and re-resolve via role+name+nth identity.
                tracing::debug!(
                    role = %r.role,
                    name = %r.name,
                    nth = r.nth,
                    "snapshot ref stale, attempting re-resolution"
                );
                let ax = crate::aria::axtree::extract_ax_tree(&page).await?;
                let fresh = ax
                    .refs
                    .iter()
                    .find(|f| f.role == r.role && f.name == r.name && f.nth == r.nth)
                    .cloned()
                    .ok_or_else(|| Error::ToolFailed {
                        tool: "snapshot",
                        message: format!(
                            "ref {} (role={}, name={:?}) could not be re-resolved",
                            i, r.role, r.name
                        ),
                    })?;
                self.store_snapshot_refs(ax.refs);
                try_tag(&page, &fresh).await
            }
        }
    }
}

async fn try_tag(page: &chromiumoxide::Page, r: &AxRef) -> Result<String> {
    let sel = tag_and_get_selector(page, r.backend_node_id).await?;
    page.find_element(sel.clone()).await.map_err(Error::Cdp)?;
    Ok(sel)
}

async fn tag_and_get_selector(
    page: &chromiumoxide::Page,
    bid: BackendNodeId,
) -> Result<String> {
    let resolved = page
        .execute(ResolveNodeParams::builder().backend_node_id(bid).build())
        .await
        .map_err(Error::Cdp)?;
    let object_id = resolved
        .result
        .object
        .object_id
        .clone()
        .ok_or_else(|| Error::ToolFailed {
            tool: "snapshot",
            message: "resolveNode returned no object id".into(),
        })?;
    let response = page
        .execute(
            CallFunctionOnParams::builder()
                .object_id(object_id)
                .return_by_value(true)
                .function_declaration(
                    "function() {\
                        const a = 'data-__fb-ref';\
                        if (!this.hasAttribute || !this.setAttribute) { return null; }\
                        if (!this.hasAttribute(a)) {\
                            const v = 'r' + Math.random().toString(36).slice(2, 10);\
                            this.setAttribute(a, v);\
                        }\
                        return '[' + a + '=\"' + this.getAttribute(a) + '\"]';\
                    }",
                )
                .build()
                .map_err(|e| Error::ToolFailed {
                    tool: "snapshot",
                    message: e.to_string(),
                })?,
        )
        .await
        .map_err(Error::Cdp)?;
    response
        .result
        .result
        .value
        .as_ref()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Error::ToolFailed {
            tool: "snapshot",
            message: "failed to tag element".into(),
        })
}

