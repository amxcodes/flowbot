pub mod telegram;
pub mod templates;
pub mod wizard;
pub mod workspace_mgmt;

use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    pub workspace_dir: Option<PathBuf>,
    pub skip_wizard: bool,
}

/// Run the interactive setup wizard
pub async fn run_setup_wizard(opts: SetupOptions) -> Result<()> {
    wizard::interactive_setup(opts).await
}

/// Basic setup without wizard (creates default workspace)
pub async fn basic_setup(opts: SetupOptions) -> Result<()> {
    let workspace_dir = opts.workspace_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".flowbot")
            .join("workspace")
    });

    workspace_mgmt::create_default_workspace(&workspace_dir).await?;

    println!("✅ Workspace created at: {}", workspace_dir.display());
    println!("   Run 'flowbot setup --wizard' for interactive configuration");

    Ok(())
}
