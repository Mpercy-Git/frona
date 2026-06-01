use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::Page;
use chromiumoxide::browser::Browser;
use chromiumoxide::handler::HandlerConfig;
use futures::StreamExt;
use tokio::task::JoinHandle;

use crate::Result;
use crate::aria::axtree::AxRef;
use crate::error::Error;

struct Handle {
    browser: Browser,
    handler_task: Mutex<Option<JoinHandle<()>>>,
    keepalive_task: Mutex<Option<JoinHandle<()>>>,
    snapshot_refs: Mutex<Option<Vec<AxRef>>>,
    last_snapshot: Mutex<Option<String>>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        for slot in [&self.handler_task, &self.keepalive_task] {
            if let Ok(mut guard) = slot.lock()
                && let Some(task) = guard.take()
            {
                task.abort();
            }
        }
    }
}

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);

#[derive(Clone)]
pub struct BrowserConnection {
    inner: Arc<Handle>,
}

impl BrowserConnection {
    pub async fn connect(ws_url: &str, timeout: Duration) -> Result<Self> {
        let config = HandlerConfig {
            request_timeout: timeout,
            ..Default::default()
        };
        let (browser, mut handler) = Browser::connect_with_config(ws_url, config)
            .await
            .map_err(Error::Cdp)?;

        let handler_task = tokio::spawn(async move {
            while let Some(res) = handler.next().await {
                if let Err(e) = res {
                    tracing::debug!(error = %e, "chromiumoxide handler event error");
                }
            }
        });

        let inner = Arc::new(Handle {
            browser,
            handler_task: Mutex::new(Some(handler_task)),
            keepalive_task: Mutex::new(None),
            snapshot_refs: Mutex::new(None),
            last_snapshot: Mutex::new(None),
        });
        let conn = BrowserConnection { inner };
        if conn.pages().await?.is_empty() {
            conn.inner
                .browser
                .new_page("about:blank")
                .await
                .map_err(Error::Cdp)?;
        }
        conn.spawn_keepalive();
        Ok(conn)
    }

    fn spawn_keepalive(&self) {
        let weak = Arc::downgrade(&self.inner);
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(KEEPALIVE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let pages = inner.browser.pages().await;
                match pages {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "keepalive ping failed; stopping");
                        return;
                    }
                }
            }
        });
        if let Ok(mut guard) = self.inner.keepalive_task.lock() {
            *guard = Some(task);
        }
    }

    pub(crate) async fn active_page(&self) -> Result<Page> {
        let pages = self.inner.browser.pages().await.map_err(Error::Cdp)?;
        if pages.is_empty() {
            return Err(Error::NoActivePage);
        }
        for page in &pages {
            let visible = page
                .evaluate("document.visibilityState === 'visible' && document.hasFocus()")
                .await
                .ok()
                .and_then(|r| r.value().and_then(|v| v.as_bool()))
                .unwrap_or(false);
            if visible {
                return Ok(page.clone());
            }
        }
        Ok(pages.into_iter().next().unwrap())
    }

    pub(crate) async fn pages(&self) -> Result<Vec<Page>> {
        self.inner.browser.pages().await.map_err(Error::Cdp)
    }

    pub(crate) fn browser(&self) -> &Browser {
        &self.inner.browser
    }

    pub(crate) fn store_snapshot_refs(&self, refs: Vec<AxRef>) {
        if let Ok(mut guard) = self.inner.snapshot_refs.lock() {
            *guard = Some(refs);
        }
    }

    pub(crate) fn lookup_snapshot_ref(&self, index: usize) -> Result<AxRef> {
        self.inner
            .snapshot_refs
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|v| v.get(index).cloned()))
            .ok_or(Error::UnknownSnapshotIndex(index))
    }

    pub(crate) fn take_last_snapshot(&self) -> Option<String> {
        self.inner.last_snapshot.lock().ok().and_then(|g| g.clone())
    }

    pub(crate) fn store_last_snapshot(&self, rendered: String) {
        if let Ok(mut guard) = self.inner.last_snapshot.lock() {
            *guard = Some(rendered);
        }
    }

    pub async fn disconnect(self) -> Result<()> {
        for slot in [&self.inner.handler_task, &self.inner.keepalive_task] {
            if let Ok(mut guard) = slot.lock()
                && let Some(task) = guard.take()
            {
                task.abort();
            }
        }
        Ok(())
    }
}
