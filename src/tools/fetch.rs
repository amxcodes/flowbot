use anyhow::Result;
use reqwest::header::USER_AGENT;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchArgs {
    pub url: String,
    pub extract_mode: Option<String>, // "markdown" or "text"
}

pub async fn web_fetch(args: WebFetchArgs) -> Result<String> {
    let client = reqwest::Client::new();
    
    let res = client
        .get(&args.url)
        .header(USER_AGENT, "FlowBot/1.0 (Mozilla/5.0)")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(anyhow::anyhow!("Request failed with status: {}", res.status()));
    }

    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Size check
    if let Some(len) = res.content_length()
        && len > 5 * 1024 * 1024 { // 5MB limit
             return Err(anyhow::anyhow!("Content too large ({} bytes). Limit is 5MB.", len));
        }

    let body = res.text().await?;
    
    if body.len() > 5 * 1024 * 1024 {
        return Err(anyhow::anyhow!("Downloaded content exceeds 5MB limit."));
    }

    // HTML Processing
    if content_type.contains("text/html") {
        let document = Html::parse_document(&body);
        
        // Remove noise
        let _noise_selector = Selector::parse("script, style, nav, footer, iframe, svg, noscript").unwrap();
         // Note: Scraper doesn't support easy removal. 
         // Strategy: Select ALL text nodes, filter if they are children of noise tags? 
         // Easier: Use a crate like `readability` or heuristic. 
         // For now, heuristic: Select p, h1-h6, li, pre, code.
        
        let content_selector = Selector::parse("body p, body h1, body h2, body h3, body h4, body li, body pre, body code, body article").unwrap();
        
        let mut text_parts = Vec::new();
        
        for element in document.select(&content_selector) {
            let text = element.text().collect::<Vec<_>>().join(" ");
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                // If markdown mode (fake simple markdown)
                if args.extract_mode.as_deref() == Some("markdown") {
                     let tag = element.value().name();
                     match tag {
                         "h1" => text_parts.push(format!("# {}\n", trimmed)),
                         "h2" => text_parts.push(format!("## {}\n", trimmed)),
                         "h3" => text_parts.push(format!("### {}\n", trimmed)),
                         "li" => text_parts.push(format!("- {}\n", trimmed)),
                         "pre" | "code" => text_parts.push(format!("```\n{}\n```\n", trimmed)),
                         _ => text_parts.push(format!("{}\n", trimmed)),
                     }
                } else {
                    text_parts.push(trimmed.to_string());
                }
            }
        }
        
        let final_text = text_parts.join("\n");
        
        // Truncate
        if final_text.len() > 15000 {
            return Ok(format!("(Truncated) {}\n...\n[Remaining {} chars truncated]", &final_text[..15000], final_text.len() - 15000));
        }
        
        if final_text.is_empty() {
             return Ok(body); // Fallback to raw if extraction failed
        }
        
        Ok(final_text)
    } else {
        // Plain text
         if body.len() > 15000 {
            return Ok(format!("(Truncated) {}\n...", &body[..15000]));
        }
        Ok(body)
    }
}
