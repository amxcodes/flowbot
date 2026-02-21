use anyhow::Result;
use reqwest::header::USER_AGENT;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

fn sanitize_prompt_injection_text(input: &str) -> (String, usize) {
    let cleaned = input.replace(['\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}'], "");

    let deny_patterns = [
        "ignore previous instructions",
        "ignore all previous instructions",
        "disregard previous instructions",
        "system prompt",
        "developer message",
        "do not reveal",
        "you are chatgpt",
        "override your instructions",
        "jailbreak",
        "prompt injection",
    ];

    let mut removed = 0usize;
    let mut kept = Vec::new();
    for line in cleaned.lines() {
        let l = line.trim().to_ascii_lowercase();
        if deny_patterns.iter().any(|p| l.contains(p)) {
            removed += 1;
            continue;
        }
        kept.push(line);
    }

    (kept.join("\n"), removed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebFetchArgs {
    pub url: String,
    pub extract_mode: Option<String>, // "markdown" or "text"
}

pub(super) async fn web_fetch(_token: &super::ExecutorToken, args: WebFetchArgs) -> Result<String> {
    let client = reqwest::Client::new();

    let res = client
        .get(&args.url)
        .header(USER_AGENT, "Nanobot/1.0 (Mozilla/5.0)")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(anyhow::anyhow!(
            "Request failed with status: {}",
            res.status()
        ));
    }

    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Size check
    if let Some(len) = res.content_length()
        && len > 5 * 1024 * 1024
    {
        // 5MB limit
        return Err(anyhow::anyhow!(
            "Content too large ({} bytes). Limit is 5MB.",
            len
        ));
    }

    let body = res.text().await?;

    if body.len() > 5 * 1024 * 1024 {
        return Err(anyhow::anyhow!("Downloaded content exceeds 5MB limit."));
    }

    // HTML Processing
    if content_type.contains("text/html") {
        let document = Html::parse_document(&body);

        // Remove noise
        let _noise_selector = Selector::parse("script, style, nav, footer, iframe, svg, noscript")
            .expect("Valid CSS selector");
        // Note: Scraper doesn't support easy removal.
        // Strategy: Select ALL text nodes, filter if they are children of noise tags?
        // Easier: Use a crate like `readability` or heuristic.
        // For now, heuristic: Select p, h1-h6, li, pre, code.

        let content_selector = Selector::parse("body p, body h1, body h2, body h3, body h4, body li, body pre, body code, body article").expect("Valid CSS selector");

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
        let (sanitized_text, removed_lines) = sanitize_prompt_injection_text(&final_text);
        let final_text = if removed_lines > 0 {
            format!(
                "[Sanitization: removed {} suspicious instruction line(s)]\n\n{}",
                removed_lines, sanitized_text
            )
        } else {
            sanitized_text
        };

        // Truncate
        if final_text.len() > 15000 {
            return Ok(format!(
                "(Truncated) {}\n...\n[Remaining {} chars truncated]",
                &final_text[..15000],
                final_text.len() - 15000
            ));
        }

        if final_text.is_empty() {
            let (sanitized_body, removed_lines) = sanitize_prompt_injection_text(&body);
            if removed_lines > 0 {
                return Ok(format!(
                    "[Sanitization: removed {} suspicious instruction line(s)]\n\n{}",
                    removed_lines, sanitized_body
                ));
            }
            return Ok(sanitized_body); // Fallback to sanitized raw if extraction failed
        }

        Ok(final_text)
    } else {
        // Plain text
        let (sanitized_body, removed_lines) = sanitize_prompt_injection_text(&body);
        if sanitized_body.len() > 15000 {
            let prefix = if removed_lines > 0 {
                format!(
                    "[Sanitization: removed {} suspicious instruction line(s)]\n\n",
                    removed_lines
                )
            } else {
                String::new()
            };
            return Ok(format!(
                "{}(Truncated) {}\n...",
                prefix,
                &sanitized_body[..15000]
            ));
        }
        if removed_lines > 0 {
            Ok(format!(
                "[Sanitization: removed {} suspicious instruction line(s)]\n\n{}",
                removed_lines, sanitized_body
            ))
        } else {
            Ok(sanitized_body)
        }
    }
}
