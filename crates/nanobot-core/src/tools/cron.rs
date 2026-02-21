use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::cron::{CronJob, CronScheduler, Payload, Schedule, SessionTarget};

/// Actions for the cron tool
pub enum CronAction {
    Status,
    List,
    Add,
    Update,
    Remove,
    Run,
    Runs,
    Wake,
}

/// Execute a cron tool call
pub async fn execute_cron_tool(scheduler: &CronScheduler, tool_call: &Value) -> Result<String> {
    let action_str = tool_call["action"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'action' field"))?;

    let action = match action_str {
        "status" => CronAction::Status,
        "list" => CronAction::List,
        "add" => CronAction::Add,
        "update" => CronAction::Update,
        "remove" => CronAction::Remove,
        "run" => CronAction::Run,
        "runs" => CronAction::Runs,
        "wake" => CronAction::Wake,
        _ => return Err(anyhow!("Unknown action: {}", action_str)),
    };

    match action {
        CronAction::Status => {
            let status = scheduler.status().await?;
            Ok(serde_json::to_string(&status)?)
        }
        CronAction::List => {
            let include_disabled = tool_call["includeDisabled"].as_bool().unwrap_or(false);
            let jobs = scheduler.list_jobs(include_disabled)?;
            Ok(serde_json::to_string(&json!({ "jobs": jobs }))?)
        }
        CronAction::Add => {
            let job_data = tool_call["job"]
                .as_object()
                .ok_or_else(|| anyhow!("Missing 'job' field"))?;

            // Parse schedule
            let schedule_obj = job_data["schedule"]
                .as_object()
                .ok_or_else(|| anyhow!("Missing 'schedule' in job"))?;

            let schedule = match schedule_obj["kind"].as_str() {
                Some("at") => {
                    let at_ms = schedule_obj["atMs"]
                        .as_u64()
                        .ok_or_else(|| anyhow!("Missing 'atMs' in at schedule"))?;
                    Schedule::At { at_ms }
                }
                Some("every") => {
                    let every_ms = schedule_obj["everyMs"]
                        .as_u64()
                        .ok_or_else(|| anyhow!("Missing 'everyMs' in every schedule"))?;
                    let anchor_ms = schedule_obj["anchorMs"].as_u64();
                    Schedule::Every {
                        every_ms,
                        anchor_ms,
                    }
                }
                Some("cron") => {
                    let expr = schedule_obj["expr"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing 'expr' in cron schedule"))?
                        .to_string();
                    let tz = schedule_obj["tz"].as_str().map(|s| s.to_string());
                    Schedule::Cron { expr, tz }
                }
                _ => return Err(anyhow!("Invalid or missing schedule kind")),
            };

            // Parse payload
            let payload_obj = job_data["payload"]
                .as_object()
                .ok_or_else(|| anyhow!("Missing 'payload' in job"))?;

            let payload = match payload_obj["kind"].as_str() {
                Some("systemEvent") => {
                    let text = payload_obj["text"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing 'text' in systemEvent payload"))?
                        .to_string();
                    Payload::SystemEvent { text }
                }
                Some("agentTurn") => {
                    let message = payload_obj["message"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing 'message' in agentTurn payload"))?
                        .to_string();
                    let model = payload_obj["model"].as_str().map(|s| s.to_string());
                    let thinking = payload_obj["thinking"].as_str().map(|s| s.to_string());
                    let timeout_seconds = payload_obj["timeoutSeconds"].as_u64();
                    Payload::AgentTurn {
                        message,
                        model,
                        thinking,
                        timeout_seconds,
                    }
                }
                _ => return Err(anyhow!("Invalid or missing payload kind")),
            };

            // Parse session target
            let session_target = match job_data["sessionTarget"].as_str() {
                Some("main") => SessionTarget::Main,
                Some("isolated") => SessionTarget::Isolated,
                _ => return Err(anyhow!("Invalid or missing sessionTarget")),
            };

            let name = job_data["name"].as_str().map(|s| s.to_string());

            let job = CronJob::new(name, schedule, payload, session_target);
            let job_id = scheduler.add_job(job).await?;

            Ok(serde_json::to_string(
                &json!({ "id": job_id, "created": true }),
            )?)
        }
        CronAction::Update => Err(anyhow!(
            "Cron action 'update' is not implemented yet; remove and re-add the job"
        )),
        CronAction::Remove => {
            let job_id = tool_call["jobId"]
                .as_str()
                .or_else(|| tool_call["id"].as_str())
                .ok_or_else(|| anyhow!("Missing 'jobId' or 'id'"))?;

            scheduler.remove_job(job_id)?;
            Ok(json!({ "removed": true }).to_string())
        }
        CronAction::Run => {
            let job_id = tool_call["jobId"]
                .as_str()
                .or_else(|| tool_call["id"].as_str())
                .ok_or_else(|| anyhow!("Missing 'jobId' or 'id'"))?;
            scheduler.trigger_job_now(job_id).await?;
            Ok(json!({ "status": "queued", "jobId": job_id, "manual": true }).to_string())
        }
        CronAction::Runs => {
            let job_id = tool_call["jobId"]
                .as_str()
                .or_else(|| tool_call["id"].as_str())
                .ok_or_else(|| anyhow!("Missing 'jobId' or 'id'"))?;
            let limit = tool_call["limit"].as_u64().unwrap_or(20) as usize;
            let entries = scheduler.job_runs(job_id, limit)?;
            Ok(json!({
                "jobId": job_id,
                "count": entries.len(),
                "runs": entries,
            })
            .to_string())
        }
        CronAction::Wake => {
            let text = if let Some(checks) = tool_call.get("checks").and_then(|v| v.as_array()) {
                let mut lines = Vec::new();
                for c in checks {
                    if let Some(s) = c.as_str() {
                        lines.push(format!("- {}", s));
                    }
                }
                if lines.is_empty() {
                    tool_call["text"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing 'text' or non-empty 'checks'"))?
                        .to_string()
                } else {
                    format!(
                        "Heartbeat batch request:\n{}\n\nExecute all checks in a single turn and return concise results.",
                        lines.join("\n")
                    )
                }
            } else {
                tool_call["text"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'text'"))?
                    .to_string()
            };
            let mode = tool_call["mode"].as_str().unwrap_or("now");
            scheduler.wake_now(text.clone()).await?;
            Ok(json!({
                "status": "queued",
                "action": "wake",
                "mode": mode,
                "text": text,
            })
            .to_string())
        }
    }
}
