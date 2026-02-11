use anyhow::Result;
use std::path::Path;
use tokio::fs;

use super::{templates, wizard::WizardData};

/// Create workspace directory with all template files
pub async fn create_workspace(workspace_dir: &Path, data: WizardData) -> Result<()> {
    // Create directory structure
    fs::create_dir_all(workspace_dir).await?;
    fs::create_dir_all(workspace_dir.join("memory")).await?;

    // Generate file contents
    let soul = if data.personality_pending {
        templates::soul_pending_template()
    } else {
        templates::soul_template(data.personality)
    };
    let identity = if data.agent_name_pending {
        templates::identity_pending_template(&data.agent_emoji, data.personality)
    } else {
        templates::identity_template(&data.agent_name, &data.agent_emoji, data.personality)
    };
    let user = templates::user_template(&data.user_name, &data.timezone);
    let agents = templates::agents_template();
    let tools = templates::tools_template();
    let bootstrap = templates::bootstrap_template(&data.agent_name, &data.user_name);

    // Write files
    fs::write(workspace_dir.join("SOUL.md"), soul).await?;
    fs::write(workspace_dir.join("IDENTITY.md"), identity).await?;
    fs::write(workspace_dir.join("USER.md"), user).await?;
    fs::write(workspace_dir.join("AGENTS.md"), agents).await?;
    fs::write(workspace_dir.join("TOOLS.md"), tools).await?;
    fs::write(workspace_dir.join("BOOTSTRAP.md"), bootstrap).await?;

    // Create empty memory file
    fs::write(
        workspace_dir.join("memory/README.md"),
        "# Memory\n\nDaily memory logs will appear here.\n",
    )
    .await?;

    println!("  ✓ Created SOUL.md");
    println!("  ✓ Created IDENTITY.md");
    println!("  ✓ Created USER.md");
    println!("  ✓ Created AGENTS.md");
    println!("  ✓ Created TOOLS.md");
    println!("  ✓ Created BOOTSTRAP.md");
    println!("  ✓ Created memory/ directory");

    Ok(())
}

/// Create default workspace without wizard (basic setup)
pub async fn create_default_workspace(workspace_dir: &Path) -> Result<()> {
    use super::templates::Personality;

    let data = WizardData {
        agent_name: "Flowbot".to_string(),
        agent_name_pending: false,
        personality: Personality::Casual,
        personality_pending: false,
        user_name: "User".to_string(),
        timezone: "UTC".to_string(),
        channels: vec![],
        agent_emoji: "🤖".to_string(),
    };

    create_workspace(workspace_dir, data).await
}

/// Edit a workspace file in the default editor
pub async fn edit_file(file_type: &str) -> Result<()> {
    let workspace_dir = get_workspace_dir()?;

    let filename = match file_type.to_lowercase().as_str() {
        "soul" => "SOUL.md",
        "identity" => "IDENTITY.md",
        "user" => "USER.md",
        "agents" => "AGENTS.md",
        "tools" => "TOOLS.md",
        _ => {
            return Err(anyhow::anyhow!(
                "Unknown file type. Use: soul, identity, user, agents, or tools"
            ));
        }
    };

    let filepath = workspace_dir.join(filename);

    if !filepath.exists() {
        return Err(anyhow::anyhow!(
            "File not found: {}. Run 'flowbot setup' first.",
            filepath.display()
        ));
    }

    // Open in default editor
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
        if cfg!(windows) {
            "notepad".to_string()
        } else {
            "vim".to_string()
        }
    });

    println!("Opening {} in {}...", filename, editor);

    let status = std::process::Command::new(&editor)
        .arg(&filepath)
        .status()?;

    if status.success() {
        println!("✓ Saved changes to {}", filename);
    } else {
        println!("! Editor exited with error");
    }

    Ok(())
}

/// Show workspace information
pub async fn show() -> Result<()> {
    let workspace_dir = get_workspace_dir()?;

    if !workspace_dir.exists() {
        println!("No workspace found at: {}", workspace_dir.display());
        println!("Run 'flowbot setup' to create one.");
        return Ok(());
    }

    println!("Workspace: {}", workspace_dir.display());
    println!();

    // List files
    let files = vec![
        "SOUL.md",
        "IDENTITY.md",
        "USER.md",
        "AGENTS.md",
        "TOOLS.md",
        "BOOTSTRAP.md",
    ];

    for file in files {
        let path = workspace_dir.join(file);
        if path.exists() {
            let metadata = fs::metadata(&path).await?;
            let size = metadata.len();
            println!("  ✓ {} ({} bytes)", file, size);
        } else {
            println!("  ✗ {} (missing)", file);
        }
    }

    println!();
    println!("Edit files:");
    println!("  flowbot workspace:edit soul");
    println!("  flowbot workspace:edit identity");
    println!("  flowbot workspace:edit user");

    Ok(())
}

/// Reset workspace to default templates
pub async fn reset() -> Result<()> {
    use console::style;
    use dialoguer::Confirm;

    let workspace_dir = get_workspace_dir()?;

    if !workspace_dir.exists() {
        return Err(anyhow::anyhow!(
            "No workspace found. Run 'flowbot setup' first."
        ));
    }

    println!(
        "{}",
        style("⚠️  WARNING: This will overwrite all workspace files!")
            .bold()
            .red()
    );
    println!();

    let confirm = Confirm::new()
        .with_prompt("Are you sure you want to reset your workspace?")
        .default(false)
        .interact()?;

    if !confirm {
        println!("Reset cancelled.");
        return Ok(());
    }

    create_default_workspace(&workspace_dir).await?;

    println!();
    println!("{}", style("✓ Workspace reset to defaults").bold().green());

    Ok(())
}

fn get_workspace_dir() -> Result<std::path::PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
        .join(".flowbot")
        .join("workspace"))
}
