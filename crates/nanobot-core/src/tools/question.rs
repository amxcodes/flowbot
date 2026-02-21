use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionPayload {
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub multiple: bool,
}

pub fn parse_question_payload(raw: &str) -> Option<QuestionPayload> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    if value.get("type")?.as_str()? != "question" {
        return None;
    }

    Some(QuestionPayload {
        header: value
            .get("header")
            .and_then(|v| v.as_str())
            .unwrap_or("Question")
            .to_string(),
        question: value
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        options: value
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        multiple: value
            .get("multiple")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

pub fn format_question_prompt(q: &QuestionPayload) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}\n{}\n", q.header, q.question));

    for (idx, opt) in q.options.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", idx + 1, opt));
    }

    if q.multiple {
        out.push_str("Reply with one or more options (e.g. 1,3)\n");
    } else {
        out.push_str("Reply with one option number or label\n");
    }

    out
}

pub fn normalize_question_answer(q: &QuestionPayload, answer: &str) -> Result<String, String> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Err("Please provide an answer.".to_string());
    }

    if q.options.is_empty() {
        return Ok(trimmed.to_string());
    }

    if q.multiple {
        let mut selected = Vec::new();
        for token in trimmed.split([',', ';']) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            selected.push(resolve_single_option(&q.options, token)?);
        }
        if selected.is_empty() {
            return Err("Please choose at least one option.".to_string());
        }
        Ok(selected.join(", "))
    } else {
        resolve_single_option(&q.options, trimmed)
    }
}

fn resolve_single_option(options: &[String], token: &str) -> Result<String, String> {
    if let Ok(index) = token.parse::<usize>()
        && index >= 1
        && index <= options.len()
    {
        return Ok(options[index - 1].clone());
    }

    if let Some(found) = options.iter().find(|opt| opt.eq_ignore_ascii_case(token)) {
        return Ok(found.clone());
    }

    Err("Invalid option. Reply with a listed number or label.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_payload_and_normalize() {
        let raw = r#"{"type":"question","header":"Mode","question":"Choose","options":["Quick","Thorough"],"multiple":false}"#;
        let payload = parse_question_payload(raw).expect("payload should parse");
        assert_eq!(payload.header, "Mode");
        assert_eq!(normalize_question_answer(&payload, "1").unwrap(), "Quick");
        assert_eq!(
            normalize_question_answer(&payload, "thorough").unwrap(),
            "Thorough"
        );
    }
}
