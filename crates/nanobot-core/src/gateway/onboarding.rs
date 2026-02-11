use std::collections::HashMap;
use std::path::PathBuf;
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

    if has_files && !needs_name && !needs_personality {
        return Ok(OnboardingOutcome::NotNeeded);
    }

    if has_files && (needs_name || needs_personality) {
        return process_pending_completion(text, &workspace_dir, needs_name, needs_personality).await;
    }

    enum Action {
        Reply(String),
        Complete(WizardData),
    }

    let key = format!("{}:{}", channel, user_id);
    let action = {
        let mut states = ONBOARDING_STATES
            .lock()
            .map_err(|_| anyhow::anyhow!("Onboarding state lock poisoned"))?;

        let state = states.entry(key.clone()).or_insert_with(OnboardingState::new);

        if state.step == 0 {
            state.step = 1;
            Action::Reply(
                "👋 Quick setup required before I can chat here.\n\n1/5 What should I call myself? (example: Flowbot)".to_string(),
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
                            "2/5 Choose personality: 1) Professional  2) Casual  3) Chaotic Good  4) Custom  5) Skip"
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
                        Action::Reply("3/5 Pick emoji signature (example: 🤖)".to_string())
                    }
                    3 => {
                        state.agent_emoji = Some(input.to_string());
                        state.step = 4;
                        Action::Reply("4/5 What should I call you?".to_string())
                    }
                    4 => {
                        state.user_name = Some(input.to_string());
                        state.step = 5;
                        Action::Reply("5/5 Your timezone (example: UTC or Asia/Kolkata)".to_string())
                    }
                    5 => {
                        state.timezone = Some(input.to_string());
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
                    _ => {
                        state.step = 1;
                        Action::Reply("1/5 What should I call myself?".to_string())
                    }
                }
            }
        }
    };

    match action {
        Action::Reply(reply) => Ok(OnboardingOutcome::ReplyOnly(reply)),
        Action::Complete(data) => {
            crate::setup::workspace_mgmt::create_workspace(&workspace_dir, data).await?;
            Ok(OnboardingOutcome::ReplyOnly(
                "✅ Setup complete. Personality files created. Send your message again and I’ll respond normally."
                    .to_string(),
            ))
        }
    }
}

async fn process_pending_completion(
    text: &str,
    workspace_dir: &PathBuf,
    needs_name: bool,
    needs_personality: bool,
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

        return Ok(OnboardingOutcome::ReplyOnly(
            "✅ Name completed. Send your message again and I’ll respond normally.".to_string(),
        ));
    }

    if needs_personality {
        return process_personality_completion(text, workspace_dir).await;
    }

    Ok(OnboardingOutcome::NotNeeded)
}

async fn process_personality_completion(
    text: &str,
    workspace_dir: &PathBuf,
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

    Ok(OnboardingOutcome::ReplyOnly(
        "✅ Personality completed. Send your message again and I’ll respond normally.".to_string(),
    ))
}

async fn set_identity_name(workspace_dir: &PathBuf, name: &str) -> Result<()> {
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

fn workspace_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".flowbot")
        .join("workspace")
}

fn personality_files_exist(workspace_dir: &PathBuf) -> bool {
    workspace_dir.join("SOUL.md").exists()
        && workspace_dir.join("IDENTITY.md").exists()
        && workspace_dir.join("USER.md").exists()
}

async fn personality_pending(workspace_dir: &PathBuf) -> bool {
    match tokio::fs::read_to_string(workspace_dir.join("SOUL.md")).await {
        Ok(content) => content.contains("NANOBOT_PERSONALITY_PENDING"),
        Err(_) => false,
    }
}

async fn name_pending(workspace_dir: &PathBuf) -> bool {
    match tokio::fs::read_to_string(workspace_dir.join("IDENTITY.md")).await {
        Ok(content) => content.contains("NANOBOT_NAME_PENDING"),
        Err(_) => false,
    }
}
