// Web search tool using DuckDuckGo

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Arguments for web search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchArgs {
    pub query: String,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    5
}

/// Search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search the web using DuckDuckGo
pub async fn web_search(args: WebSearchArgs) -> Result<Vec<SearchResult>> {
    // DuckDuckGo HTML search
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(&args.query)
    );

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()?;

    let response = client.get(&url).send().await?;
    let html = response.text().await?;

    // Parse HTML using scraper
    use scraper::{Html, Selector};

    let document = Html::parse_document(&html);
    let result_selector = Selector::parse(".result").expect("Valid CSS selector");
    let title_selector = Selector::parse(".result__a").expect("Valid CSS selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("Valid CSS selector");
    let url_selector = Selector::parse(".result__url").expect("Valid CSS selector");

    let mut results = Vec::new();

    for element in document.select(&result_selector).take(args.max_results) {
        let title = element
            .select(&title_selector)
            .next()
            .map(|e| e.inner_html())
            .unwrap_or_default();

        let snippet = element
            .select(&snippet_selector)
            .next()
            .map(|e| e.inner_html())
            .unwrap_or_default();

        let url = element
            .select(&url_selector)
            .next()
            .map(|e| {
                // Extract URL from display text
                e.inner_html()
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .to_string()
            })
            .map(|u| format!("https://{}", u))
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_escape::decode_html_entities(&title).to_string(),
                url,
                snippet: html_escape::decode_html_entities(&snippet).to_string(),
            });
        }
    }

    if results.is_empty() {
        return Err(anyhow::anyhow!(
            "No search results found for query: {}",
            args.query
        ));
    }

    Ok(results)
}
