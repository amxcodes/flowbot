// Browser client using chromiumoxide - persistent implementation
use anyhow::{anyhow, Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig as ChromiumConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::config::BrowserConfig;

#[derive(Clone)]
pub struct BrowserClient {
    browser: Arc<Mutex<Option<Browser>>>,
    page: Arc<Mutex<Option<Page>>>,
    config: BrowserConfig,
}

impl BrowserClient {
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            browser: Arc::new(Mutex::new(None)),
            page: Arc::new(Mutex::new(None)),
            config,
        }
    }

    pub async fn launch(&self) -> Result<()> {
        let _ = self.ensure_browser().await?;
        Ok(())
    }

    async fn ensure_browser(&self) -> Result<Browser> {
        let mut browser_guard = self.browser.lock().await;
        
        if let Some(browser) = browser_guard.as_ref() {
            // TODO: Check if browser is actually alive?
            // For now assume it is.
            return Ok(browser.clone());
        }

        tracing::info!("🌐 Launching new browser instance...");
        
        let mut builder = ChromiumConfig::builder();
        
        // Headless mode (default is headless, so we only need to opt-out)
        if !self.config.headless {
            builder = builder.with_head();
        }

        // Persistence
        if let Some(user_data_dir) = &self.config.user_data_dir {
            builder = builder.user_data_dir(user_data_dir);
        }

        // Proxy
        if let Some(proxy) = &self.config.proxy {
            builder = builder.args(vec![format!("--proxy-server={}", proxy)]);
        }

        let (browser, mut handler) = Browser::launch(builder.build().map_err(|e| anyhow!("Browser config error: {}", e))?)
            .await
            .context("Failed to launch browser")?;

        // Spawn handler loop
        tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if h.is_err() {
                    tracing::debug!("Browser handler error: {:?}", h);
                    break;
                }
            }
            tracing::info!("Browser handler loop exited");
        });

        *browser_guard = Some(browser.clone());
        Ok(browser)
    }

    pub async fn get_page(&self) -> Result<Page> {
        let mut page_guard = self.page.lock().await;

        if let Some(page) = page_guard.as_ref() {
            return Ok(page.clone());
        }

        let browser = self.ensure_browser().await?;

        // Check for existing pages first
        let pages = browser.pages().await.unwrap_or_default();
        if let Some(page) = pages.first() {
             *page_guard = Some(page.clone());
             return Ok(page.clone());
        }

        // Create new page
        let page = browser.new_page("about:blank").await.context("Failed to create page")?;
        *page_guard = Some(page.clone());

        Ok(page)
    }

    pub async fn get_pages(&self) -> Result<Vec<Page>> {
         let browser = self.ensure_browser().await?;
         Ok(browser.pages().await.map_err(|e| anyhow!("Failed to get pages: {}", e))?)
    }

    pub async fn switch_tab(&self, index: usize) -> Result<Page> {
        let pages = self.get_pages().await?;
        if let Some(page) = pages.get(index) {
            let mut active = self.page.lock().await;
            *active = Some(page.clone());
            // Make visible
            let _ = page.bring_to_front().await;
            Ok(page.clone())
        } else {
            Err(anyhow!("Invalid tab index"))
        }
    }

    pub async fn navigate(&self, url: &str) -> Result<Page> {
        let page = self.get_page().await?;
        page.goto(url).await.context(format!("Failed to navigate to {}", url))?;
        // Wait for load? Chromiumoxide goto waits for load event by default usually? 
        // Docs say it returns when strictly necessary.
        // Let's add a small wait or verify load state if needed.
        Ok(page)
    }
}
