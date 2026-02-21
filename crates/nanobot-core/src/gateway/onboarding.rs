use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use once_cell::sync::Lazy;

use crate::setup::templates::{self, Personality};
use crate::setup::wizard::WizardData;

#[derive(Clone, Debug)]
struct OnboardingState {
    step: u8,
    agent_name: Option<String>,
    personality: Option<Personality>,
    user_name: Option<String>,
    timezone: Option<String>,
    agent_emoji: Option<String>,
    heartbeat_enabled: Option<bool>,
    heartbeat_schedule: Option<String>,
    heartbeat_timezone: Option<String>,
}

impl OnboardingState {
    fn new() -> Self {
        Self {
            step: 0,
            agent_name: None,
            personality: None,
            user_name: None,
            timezone: None,
            agent_emoji: None,
            heartbeat_enabled: None,
            heartbeat_schedule: None,
            heartbeat_timezone: None,
        }
    }
}

static ONBOARDING_STATES: Lazy<Mutex<HashMap<String, OnboardingState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub enum OnboardingOutcome {
    NotNeeded,
    ReplyOnly(String),
}

pub async fn process_onboarding_message(
    channel: &str,
    user_id: &str,
    text: &str,
) -> Result<OnboardingOutcome> {
    let workspace_dir = workspace_dir();
    let has_files = personality_files_exist(&workspace_dir);
    let needs_name = if has_files {
        name_pending(&workspace_dir).await
    } else {
        false
    };
    let needs_personality = if has_files {
        personality_pending(&workspace_dir).await
    } else {
        false
    };
    let needs_heartbeat = if has_files {
        heartbeat_pending_or_missing(&workspace_dir).await
    } else {
        false
    };

    if has_files && !needs_name && !needs_personality && !needs_heartbeat {
        return Ok(OnboardingOutcome::NotNeeded);
    }

    if has_files && (needs_name || needs_personality || needs_heartbeat) {
        return process_pending_completion(
            text,
            &workspace_dir,
            needs_name,
            needs_personality,
            needs_heartbeat,
        )
        .await;
    }

    enum Action {
        Reply(String),
        Complete(WizardData),
        CompleteWithHeartbeat {
            data: WizardData,
            enabled: bool,
            schedule: String,
            timezone: String,
        },
    }

    let key = format!("{}:{}", channel, user_id);
    let action = {
        let mut states = ONBOARDING_STATES
            .lock()
            .map_err(|_| anyhow::anyhow!("Onboarding state lock poisoned"))?;

        let state = states
            .entry(key.clone())
            .or_insert_with(OnboardingState::new);

        if state.step == 0 {
            state.step = 1;
            Action::Reply(
                "👋 Quick setup required before I can chat here.\n\n1/8 What should I call myself? (example: Flowbot)".to_string(),
            )
        } else {
            let input = text.trim();
            if input.is_empty() {
                Action::Reply("Please send a value to continue setup.".to_string())
            } else {
                match state.step {
                    1 => {
                        state.agent_name = Some(input.to_string());
                        state.step = 2;
                        Action::Reply(
                            "2/8 Choose personality: 1) Professional  2) Casual  3) Chaotic Good  4) Custom  5) Skip"
                                .to_string(),
                        )
                    }
                    2 => {
                        let personality = parse_personality(input).ok_or_else(|| {
                            anyhow::anyhow!(
                                "Please choose: 1/2/3/4/5 (or professional/casual/chaotic/custom/skip)"
                            )
                        })?;
                        state.personality = Some(personality);
                        state.step = 3;
                        Action::Reply("3/8 Pick emoji signature (example: 🤖)".to_string())
                    }
                    3 => {
                        state.agent_emoji = Some(input.to_string());
                        state.step = 4;
                        Action::Reply("4/8 What should I call you?".to_string())
                    }
                    4 => {
                        state.user_name = Some(input.to_string());
                        state.step = 5;
                        Action::Reply(
                            "5/8 Your timezone (example: UTC or Asia/Kolkata)".to_string(),
                        )
                    }
                    5 => {
                        state.timezone = Some(input.to_string());
                        state.step = 6;
                        Action::Reply("6/8 Enable proactive heartbeat tasks? (yes/no)".to_string())
                    }
                    6 => {
                        let enabled = parse_yes_no(input)
                            .ok_or_else(|| anyhow::anyhow!("Please answer yes/no (or y/n)"))?;
                        state.heartbeat_enabled = Some(enabled);

                        if enabled {
                            state.step = 7;
                            Action::Reply(
                                "7/8 Heartbeat cron schedule? (example: 0 9 * * *)".to_string(),
                            )
                        } else {
                            state.heartbeat_schedule = Some("0 9 * * *".to_string());
                            state.heartbeat_timezone =
                                Some(state.timezone.clone().unwrap_or_else(|| "UTC".to_string()));

                            let data = WizardData {
                                agent_name: state
                                    .agent_name
                                    .clone()
                                    .unwrap_or_else(|| "Flowbot".to_string()),
                                agent_name_pending: false,
                                personality: state.personality.unwrap_or(Personality::Casual),
                                user_name: state
                                    .user_name
                                    .clone()
                                    .unwrap_or_else(|| "User".to_string()),
                                timezone: state
                                    .timezone
                                    .clone()
                                    .unwrap_or_else(|| "UTC".to_string()),
                                channels: vec![channel.to_string()],
                                personality_pending: false,
                                agent_emoji: state
                                    .agent_emoji
                                    .clone()
                                    .unwrap_or_else(|| "🤖".to_string()),
                            };
                            states.remove(&key);
                            Action::Complete(data)
                        }
                    }
                    7 => {
                        state.heartbeat_schedule = Some(input.to_string());
                        state.step = 8;
                        Action::Reply(
                            "8/8 Heartbeat timezone (example: UTC or Asia/Kolkata)".to_string(),
                        )
                    }
                    8 => {
                        state.heartbeat_timezone = Some(input.to_string());
                        let data = WizardData {
                            agent_name: state
                                .agent_name
                                .clone()
                                .unwrap_or_else(|| "Flowbot".to_string()),
                            agent_name_pending: false,
                            personality: state.personality.unwrap_or(Personality::Casual),
                            user_name: state
                                .user_name
                                .clone()
                                .unwrap_or_else(|| "User".to_string()),
                            timezone: state.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
                            channels: vec![channel.to_string()],
                            personality_pending: false,
                            agent_emoji: state
                                .agent_emoji
                                .clone()
                                .unwrap_or_else(|| "🤖".to_string()),
                        };
                        let heartbeat_enabled = state.heartbeat_enabled.unwrap_or(true);
                        let heartbeat_schedule = state
                            .heartbeat_schedule
                            .clone()
                            .unwrap_or_else(|| "0 9 * * *".to_string());
                        let heartbeat_timezone =
                            state.heartbeat_timezone.clone().unwrap_or_else(|| {
                                state.timezone.clone().unwrap_or_else(|| "UTC".to_string())
                            });
                        states.remove(&key);
                        Action::CompleteWithHeartbeat {
                            data,
                            enabled: heartbeat_enabled,
                            schedule: heartbeat_schedule,
                            timezone: heartbeat_timezone,
                        }
                    }
                    _ => {
                        state.step = 1;
                        Action::Reply("1/8 What should I call myself?".to_string())
                    }
                }
            }
        }
    };

    match action {
        Action::Reply(reply) => Ok(OnboardingOutcome::ReplyOnly(reply)),
        Action::Complete(data) => {
            crate::setup::workspace_mgmt::create_workspace(&workspace_dir, data).await?;
            write_heartbeat(
                &workspace_dir,
                false,
                "0 9 * * *",
                &read_user_timezone(&workspace_dir).await,
            )
            .await?;
            Ok(OnboardingOutcome::ReplyOnly(completion_ready_message()))
        }
        Action::CompleteWithHeartbeat {
            data,
            enabled,
            schedule,
            timezone,
        } => {
            crate::setup::workspace_mgmt::create_workspace(&workspace_dir, data).await?;
            write_heartbeat(&workspace_dir, enabled, &schedule, &timezone).await?;
            Ok(OnboardingOutcome::ReplyOnly(completion_ready_message()))
        }
    }
}

async fn process_pending_completion(
    text: &str,
    workspace_dir: &Path,
    needs_name: bool,
    needs_personality: bool,
    needs_heartbeat: bool,
) -> Result<OnboardingOutcome> {
    if needs_name {
        let input = text.trim();
        if input.is_empty() {
            return Ok(OnboardingOutcome::ReplyOnly(
                "👋 Name setup is required first. What should I call myself?".to_string(),
            ));
        }

        set_identity_name(workspace_dir, input).await?;

        if needs_personality {
            return Ok(OnboardingOutcome::ReplyOnly(
                "✅ Name saved. Now choose personality: 1) Professional  2) Casual  3) Chaotic Good  4) Custom"
                    .to_string(),
            ));
        }

        return Ok(OnboardingOutcome::ReplyOnly(completion_ready_message()));
    }

    if needs_personality {
        return process_personality_completion(text, workspace_dir).await;
    }

    if needs_heartbeat {
        return process_heartbeat_completion(text, workspace_dir).await;
    }

    Ok(OnboardingOutcome::NotNeeded)
}

async fn process_heartbeat_completion(
    text: &str,
    workspace_dir: &Path,
) -> Result<OnboardingOutcome> {
    let input = text.trim();
    if input.is_empty() {
        return Ok(OnboardingOutcome::ReplyOnly(
            "👋 Heartbeat setup is required first. Enable proactive heartbeat tasks? (yes/no)"
                .to_string(),
        ));
    }

    let enabled = match parse_yes_no(input) {
        Some(v) => v,
        None => {
            return Ok(OnboardingOutcome::ReplyOnly(
                "Please answer yes/no (or y/n) for heartbeat setup.".to_string(),
            ));
        }
    };

    let timezone = read_user_timezone(workspace_dir).await;
    write_heartbeat(workspace_dir, enabled, "0 9 * * *", &timezone).await?;

    Ok(OnboardingOutcome::ReplyOnly(completion_ready_message()))
}

async fn process_personality_completion(
    text: &str,
    workspace_dir: &Path,
) -> Result<OnboardingOutcome> {
    let input = text.trim();
    if input.is_empty() {
        return Ok(OnboardingOutcome::ReplyOnly(
            "👋 Personality setup is required first. Choose: 1) Professional  2) Casual  3) Chaotic Good  4) Custom"
                .to_string(),
        ));
    }

    let personality = match input.to_lowercase().as_str() {
        "1" | "professional" => Some(Personality::Professional),
        "2" | "casual" => Some(Personality::Casual),
        "3" | "chaotic" | "chaotic good" => Some(Personality::ChaoticGood),
        "4" | "custom" => Some(Personality::Custom),
        _ => None,
    };

    let Some(personality) = personality else {
        return Ok(OnboardingOutcome::ReplyOnly(
            "Please choose personality: 1/2/3/4 (or professional/casual/chaotic/custom)."
                .to_string(),
        ));
    };

    let soul = templates::soul_template(personality);
    tokio::fs::write(workspace_dir.join("SOUL.md"), soul).await?;

    Ok(OnboardingOutcome::ReplyOnly(completion_ready_message()))
}

fn completion_ready_message() -> String {
    "✅ Setup complete. Send your message again and I’ll respond normally.\n\nTip: run 'nanobot doctor' to verify skill dependencies (deno/gh; node is optional for legacy fallbacks).".to_string()
}

async fn set_identity_name(workspace_dir: &Path, name: &str) -> Result<()> {
    let path = workspace_dir.join("IDENTITY.md");
    let content = tokio::fs::read_to_string(&path).await?;
    let mut lines: Vec<String> = Vec::new();

    for line in content.lines() {
        if line.contains("NANOBOT_NAME_PENDING") {
            continue;
        }
        if line.trim_start().starts_with("- **Name:**") {
            lines.push(format!("- **Name:** {}", name));
        } else {
            lines.push(line.to_string());
        }
    }

    tokio::fs::write(path, format!("{}\n", lines.join("\n"))).await?;
    Ok(())
}

fn parse_personality(input: &str) -> Option<Personality> {
    match input.trim().to_lowercase().as_str() {
        "1" | "professional" => Some(Personality::Professional),
        "2" | "casual" => Some(Personality::Casual),
        "3" | "chaotic" | "chaotic good" => Some(Personality::ChaoticGood),
        "4" | "custom" => Some(Personality::Custom),
        "5" | "skip" => Some(Personality::Casual),
        _ => None,
    }
}

fn parse_yes_no(input: &str) -> Option<bool> {
    match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "true" | "1" => Some(true),
        "n" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn workspace_dir() -> PathBuf {
    crate::workspace::resolve_workspace_dir()
}

fn personality_files_exist(workspace_dir: &Path) -> bool {
    workspace_dir.join("SOUL.md").exists()
        && workspace_dir.join("IDENTITY.md").exists()
        && workspace_dir.join("USER.md").exists()
}

async fn personality_pending(workspace_dir: &Path) -> bool {
    match tokio::fs::read_to_string(workspace_dir.join("SOUL.md")).await {
        Ok(content) => content.contains("NANOBOT_PERSONALITY_PENDING"),
        Err(_) => false,
    }
}

async fn name_pending(workspace_dir: &Path) -> bool {
    match tokio::fs::read_to_string(workspace_dir.join("IDENTITY.md")).await {
        Ok(content) => content.contains("NANOBOT_NAME_PENDING"),
        Err(_) => false,
    }
}

async fn heartbeat_pending_or_missing(workspace_dir: &Path) -> bool {
    let path = workspace_dir.join("HEARTBEAT.md");
    if !path.exists() {
        return true;
    }

    match tokio::fs::read_to_string(path).await {
        Ok(content) => content.contains("NANOBOT_HEARTBEAT_PENDING"),
        Err(_) => true,
    }
}

async fn read_user_timezone(workspace_dir: &Path) -> String {
    let path = workspace_dir.join("USER.md");
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return "UTC".to_string();
    };

    for line in content.lines() {
        if let Some(rest) = line.trim().strip_prefix("- **Timezone:**") {
            let tz = rest.trim();
            if !tz.is_empty() {
                return tz.to_string();
            }
        }
    }

    "UTC".to_string()
}

async fn write_heartbeat(
    workspace_dir: &Path,
    enabled: bool,
    schedule: &str,
    timezone: &str,
) -> Result<()> {
    let content = templates::heartbeat_template(enabled, schedule, timezone);
    tokio::fs::write(workspace_dir.join("HEARTBEAT.md"), content).await?;
    Ok(())
}
