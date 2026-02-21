// Browser actions (navigate, click, type, screenshot, etc.)
use anyhow::{Result, anyhow};
use chromiumoxide::cdp::js_protocol::runtime::RemoteObject;
use chromiumoxide::page::Page;

pub struct BrowserActions;

impl BrowserActions {
    /// Click an element by CSS selector
    pub async fn click(page: &Page, selector: &str) -> Result<String> {
        let element = page
            .find_element(selector)
            .await
            .map_err(|e| anyhow!("Element '{}' not found: {}", selector, e))?;

        element
            .click()
            .await
            .map_err(|e| anyhow!("Click failed: {}", e))?;

        Ok(format!("✅ Clicked element: {}", selector))
    }

    /// Type text into an element
    pub async fn type_text(page: &Page, selector: &str, text: &str) -> Result<String> {
        let element = page
            .find_element(selector)
            .await
            .map_err(|e| anyhow!("Element '{}' not found: {}", selector, e))?;

        element
            .click()
            .await
            .map_err(|e| anyhow!("Focus failed: {}", e))?;

        element
            .type_str(text)
            .await
            .map_err(|e| anyhow!("Type failed: {}", e))?;

        Ok(format!("✅ Typed '{}' into: {}", text, selector))
    }

    /// Take a screenshot of the entire page (PNG format)
    pub async fn screenshot(page: &Page) -> Result<Vec<u8>> {
        use chromiumoxide::cdp::browser_protocol::page::{
            CaptureScreenshotFormat, CaptureScreenshotParams,
        };

        let params = CaptureScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .build();

        let screenshot = page
            .screenshot(params)
            .await
            .map_err(|e| anyhow!("Screenshot failed: {}", e))?;

        Ok(screenshot)
    }

    /// Print page to PDF
    pub async fn print_to_pdf(page: &Page) -> Result<Vec<u8>> {
        use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;

        let pdf = page
            .pdf(PrintToPdfParams::builder().build())
            .await
            .map_err(|e| anyhow!("PDF generation failed: {}", e))?;

        Ok(pdf)
    }

    /// Execute JavaScript code and get result as JSON string
    pub async fn execute_js(page: &Page, script: &str) -> Result<String> {
        // Evaluate returns a RemoteObject which we can serialize
        let result: RemoteObject = page
            .evaluate(script)
            .await
            .map_err(|e| anyhow!("Script execution failed: {}", e))?
            .into_value()
            .map_err(|e| anyhow!("Failed to get script result: {}", e))?;

        // Serialize the remote object to JSON
        Ok(serde_json::to_string_pretty(&result)?)
    }

    /// Get the page HTML
    pub async fn get_html(page: &Page) -> Result<String> {
        let html = page
            .content()
            .await
            .map_err(|e| anyhow!("Failed to get page HTML: {}", e))?;

        Ok(html)
    }

    /// Get the page title
    pub async fn get_title(page: &Page) -> Result<String> {
        let title = page
            .get_title()
            .await
            .map_err(|e| anyhow!("Failed to get title: {}", e))?;

        Ok(title.unwrap_or_default())
    }
}
