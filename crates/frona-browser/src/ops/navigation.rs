use crate::Result;
use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::types::PageInfo;
use crate::url::normalize_url;

impl BrowserConnection {
    pub async fn navigate(&self, url: &str, wait_for_load: bool) -> Result<PageInfo> {
        let url = normalize_url(url);
        let page = self.active_page().await?;
        page.goto(&url).await.map_err(Error::Cdp)?;
        if wait_for_load {
            page.wait_for_navigation().await.map_err(Error::Cdp)?;
        }
        page_info(&page).await
    }

    pub async fn go_back(&self) -> Result<()> {
        let page = self.active_page().await?;
        page.evaluate("window.history.back()")
            .await
            .map_err(Error::Cdp)?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Ok(())
    }

    pub async fn go_forward(&self) -> Result<()> {
        let page = self.active_page().await?;
        page.evaluate("window.history.forward()")
            .await
            .map_err(Error::Cdp)?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Ok(())
    }
}

async fn page_info(page: &chromiumoxide::Page) -> Result<PageInfo> {
    let url = page.url().await.map_err(Error::Cdp)?.unwrap_or_default();
    let title = page
        .get_title()
        .await
        .map_err(Error::Cdp)?
        .unwrap_or_default();
    Ok(PageInfo { url, title })
}
