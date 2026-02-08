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
    browser: Arc<Mutex<Option<Arc<Browser>>>>,
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

    async fn ensure_browser(&self) -> Result<Arc<Browser>> {
        let mut browser_guard = self.browser.lock().await;
        
        if let Some(browser) = browser_guard.as_ref() {
            // TODO: Check if browser is actually alive?
            return Ok(browser.clone());
        }

        tracing::info!("🌐 Launching new browser instance...");

        let (browser, mut handler) = if self.config.use_docker {
            // Docker Logic
            let port = self.config.docker_port;
            let image = &self.config.docker_image;
            let container_name = "nanobot-browser";

            // 1. Check if container exists/running
            let status = std::process::Command::new("docker")
                .args(["inspect", "-f", "{{.State.Running}}", container_name])
                .output();

            let needs_start = match status {
                Ok(output) => {
                    let s = String::from_utf8_lossy(&output.stdout);
                    if s.trim() == "true" {
                        false // Already running
                    } else {
                        // Exists but stopped, or doesn't exist (stderr)
                        // Should probably remove and run fresh to be safe
                        let _ = std::process::Command::new("docker").args(["rm", "-f", container_name]).output();
                        true
                    }
                }
                Err(_) => true, // Docker command failed? Assume start fresh
            };

            if needs_start {
                tracing::info!("🐳 Starting Docker browser container ({}) on port {}...", image, port);
                let _ = std::process::Command::new("docker")
                    .args([
                        "run", "-d",
                        "-p", &format!("{}:9222", port),
                        "--name", container_name,
                        "--shm-size=2gb", // Prevent crashes
                        image,
                        "--remote-debugging-port=9222",
                        "--remote-debugging-address=0.0.0.0"
                    ])
                    .output()
                    .context("Failed to start docker container")?;
                
                // Wait for container to boot
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            }

            // Connect using CDP
            let url = format!("ws://localhost:{}", port);
            tracing::info!("🔌 Connecting to Docker browser at {}", url);
            
            Browser::connect(&url).await.context("Failed to connect to Docker browser")?
        } else {
            // Local fallback logic
            let mut builder = ChromiumConfig::builder();
            
            if !self.config.headless {
                builder = builder.with_head();
            }

            if let Some(user_data_dir) = &self.config.user_data_dir {
                builder = builder.user_data_dir(user_data_dir);
            }

            if let Some(proxy) = &self.config.proxy {
                builder = builder.args(vec![format!("--proxy-server={}", proxy)]);
            }

            Browser::launch(builder.build().map_err(|e| anyhow!("Browser config error: {}", e))?)
                .await
                .context("Failed to launch local browser")?
        };
        
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

        let browser_arc = Arc::new(browser);
        *browser_guard = Some(browser_arc.clone());
        Ok(browser_arc)
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
