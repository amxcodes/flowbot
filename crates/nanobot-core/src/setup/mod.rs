pub mod channel_instructions;
pub mod discord;
pub mod offline_models;
pub mod slack;
pub mod telegram;
pub mod templates;
pub mod wizard;
pub mod workspace_mgmt;

use anyhow::Result;
use std::path::PathBuf;

// Re-export wizard types
pub use wizard::SetupResult;

#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    pub workspace_dir: Option<PathBuf>,
    pub skip_wizard: bool,
}

/// Run the interactive setup wizard
pub async fn run_setup_wizard(opts: SetupOptions) -> Result<SetupResult> {
    let result = wizard::interactive_setup(opts).await?;

    // Generate config.toml based on wizard result
    generate_config(&result).await?;

    Ok(result)
}

/// Generate config.toml from wizard result
async fn generate_config(result: &SetupResult) -> Result<()> {
    use crate::config::{AntigravityConfig, BrowserConfig, Config, InteractionPolicy, Providers};

    println!("📝 Generating config.toml...");

    let existing_config = Config::load().ok();

    // 1. Determine Provider Config
    let mut providers = existing_config
        .as_ref()
        .map(|c| c.providers.clone())
        .unwrap_or(Providers {
            antigravity: None,
            openai: None,
            openrouter: None,
            telegram: None,
            teams: None,
            google_chat: None,
            google: None,
            slack: None,
            discord: None,
        });

    if let Some(provider) = &result.oauth_provider
        && provider.as_str() == "antigravity"
    {
        providers.antigravity = Some(AntigravityConfig {
            api_key: None, // OAuth uses token manager
            api_keys: None,
            base_url: None,
            fallback_base_urls: None,
        });
    }

    if let Some(webhook) = result.teams_webhook.as_ref() {
        providers.teams = Some(crate::config::TeamsConfig {
            webhook_url: webhook.clone(),
        });
    }

    if let Some(webhook) = result.google_chat_webhook.as_ref() {
        providers.google_chat = Some(crate::config::GoogleChatConfig {
            webhook_url: webhook.clone(),
        });
    }

    // 2. Browser Config
    let browser = if result.enable_browser {
        Some(BrowserConfig {
            headless: true,
            user_data_dir: None,
            proxy: None,
            use_docker: result.browser_use_docker,
            docker_image: result.browser_docker_image.clone(),
            docker_port: result.browser_docker_port,
        })
    } else {
        None
    };

    // 3. Create Config Object
    let config = Config {
        default_provider: result
            .oauth_provider
            .clone()
            .unwrap_or_else(|| "openai".to_string()),
        providers,
        llm: None,
        interaction_policy: InteractionPolicy::Interactive,
        audit_log_path: None,
        mcp: None,
        browser,
        context_token_limit: 32_000,
        session: crate::config::SessionConfig {
            dm_scope: result.dm_scope,
        },
    };

    // 4. Write to file
    // We can't use Config::save() directly because it saves to "config.toml" in CWD.
    // We want to save to workspace_dir/config.toml usually, OR CWD if that's where we are running.
    // For now, let's explicitly write to the workspace directory if it differs from CWD,
    // or just write to "config.toml" in CWD as per original design (since nanobot looks in CWD).

    // The wizard sets up the workspace at `result.workspace_dir`.
    // If we want the config to be active, it usually needs to be where we run the binary from,
    // OR we need to load it from the workspace.
    // Nanobot::load() looks at "config.toml".
    // Let's write it to CWD "config.toml" to match `config.example.toml` pattern.

    let config_path = std::path::PathBuf::from("config.toml");
    let contents = toml::to_string_pretty(&config)?;
    tokio::fs::write(&config_path, contents).await?;

    println!("✅ Config generated at: {}", config_path.display());

    Ok(())
}

/// Basic setup without wizard (creates default workspace)
pub async fn basic_setup(opts: SetupOptions) -> Result<()> {
    let workspace_dir = opts
        .workspace_dir
        .unwrap_or_else(crate::workspace::resolve_workspace_dir);

    workspace_mgmt::create_default_workspace(&workspace_dir).await?;

    println!("✅ Workspace created at: {}", workspace_dir.display());
    println!("   Run 'nanobot setup --wizard' for interactive configuration");

    Ok(())
}

pub async fn run_offline_models_installer() -> Result<()> {
    offline_models::run_offline_models_installer().await
}
