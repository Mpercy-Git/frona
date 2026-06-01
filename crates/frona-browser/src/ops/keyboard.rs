use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::keymap::build_key_events;
use crate::Result;

impl BrowserConnection {
    pub async fn press_key(&self, key: &str) -> Result<()> {
        let page = self.active_page().await?;
        let (down, up) = build_key_events(key)?;
        page.execute(down).await.map_err(Error::Cdp)?;
        page.execute(up).await.map_err(Error::Cdp)?;
        Ok(())
    }

    pub async fn scroll(&self, amount: Option<i64>) -> Result<()> {
        let page = self.active_page().await?;
        let js = match amount {
            Some(n) => format!("window.scrollBy(0, {n})"),
            None => "window.scrollTo(0, document.body.scrollHeight)".to_string(),
        };
        page.evaluate(js).await.map_err(Error::Cdp)?;
        Ok(())
    }
}
