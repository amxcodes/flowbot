pub mod telegram;
pub mod discord;
pub mod slack;
pub mod templates;
pub mod wizard;
pub mod workspace_mgmt;
pub mod channel_instructions;

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
    use crate::config::{
        Config, Providers, BrowserConfig, AntigravityConfig,
        InteractionPolicy
    };

    println!("📝 Generating config.toml...");

    // 1. Determine Provider Config
    let mut providers = Providers {
        antigravity: None,
        openai: None,
        openrouter: None,
        telegram: None, // Will be filled if token exists in env or if we want to prompt
        google: None,
    };

    if let Some(provider) = &result.oauth_provider {
        match provider.as_str() {
            "antigravity" => {
                providers.antigravity = Some(AntigravityConfig {
                    api_key: None, // OAuth uses token manager
                    api_keys: None,
                    base_url: None,
                    fallback_base_urls: None,
                });
            }
            _ => {}
        }
    }

    // 2. Browser Config
    let browser = if result.enable_browser {
        Some(BrowserConfig {
            headless: true,
            user_data_dir: None,
            proxy: None,
            use_docker: false,
            docker_image: "zenika/alpine-chrome:with-puppeteer".to_string(),
            docker_port: 9222,
        })
    } else {
        None
    };

    // 3. Create Config Object
    let config = Config {
        default_provider: result.oauth_provider.clone().unwrap_or_else(|| "openai".to_string()),
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
    let workspace_dir = opts.workspace_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
    });

    workspace_mgmt::create_default_workspace(&workspace_dir).await?;

    println!("✅ Workspace created at: {}", workspace_dir.display());
    println!("   Run 'nanobot setup --wizard' for interactive configuration");

    Ok(())
}
