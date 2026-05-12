use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::types::TabInfo;
use crate::url::normalize_url;
use crate::Result;

impl BrowserConnection {
    pub async fn new_tab(&self, url: &str) -> Result<TabInfo> {
        let target = normalize_url(url);
        let page = self
            .browser()
            .new_page(&target)
            .await
            .map_err(Error::Cdp)?;
        page.bring_to_front().await.map_err(Error::Cdp)?;

        let url = page.url().await.map_err(Error::Cdp)?.unwrap_or_default();
        let title = page.get_title().await.map_err(Error::Cdp)?.unwrap_or_default();

        let pages = self.pages().await?;
        let index = pages
            .iter()
            .position(|p| p.target_id() == page.target_id())
            .unwrap_or(pages.len().saturating_sub(1));

        Ok(TabInfo {
            index,
            url,
            title,
            active: true,
        })
    }

    pub async fn tabs(&self) -> Result<Vec<TabInfo>> {
        let pages = self.pages().await?;
        let active = self.active_page().await.ok();
        let active_id = active.as_ref().map(|p| p.target_id().clone());

        let mut out = Vec::with_capacity(pages.len());
        for (idx, page) in pages.iter().enumerate() {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            let title = page.get_title().await.ok().flatten().unwrap_or_default();
            let is_active = active_id.as_ref() == Some(page.target_id());
            out.push(TabInfo {
                index: idx,
                url,
                title,
                active: is_active,
            });
        }
        Ok(out)
    }

    pub async fn switch_tab(&self, index: usize) -> Result<()> {
        let pages = self.pages().await?;
        let page = pages.get(index).ok_or_else(|| Error::ToolFailed {
            tool: "switch_tab",
            message: format!("invalid tab index {index} (have {} tabs)", pages.len()),
        })?;
        page.bring_to_front().await.map_err(Error::Cdp)?;
        Ok(())
    }

    pub async fn close_active_tab(&self) -> Result<()> {
        let page = self.active_page().await?;
        page.close().await.map_err(Error::Cdp)?;
        Ok(())
    }
}
