use crate::config;
use crate::persistence::PersistenceManager;
use anyhow::Result;
// We'll add this for nice output, or use direct ANSI codes if keeping deps minimal
use std::path::PathBuf;
use std::collections::HashSet;

pub mod telegram; // Add this line

pub fn run_wiring_doctor() -> Result<()> {
    println!("\n🔧 Running wiring audit...\n");

    let described = extract_described_tools(&crate::tools::executor::get_tool_descriptions());
    let supported: HashSet<String> = crate::tools::executor::supported_tool_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let guarded: HashSet<String> = crate::tools::guard::ToolGuard::guarded_tool_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let described_not_supported: Vec<String> = described
        .difference(&supported)
        .cloned()
        .collect();
    let supported_not_described: Vec<String> = supported
        .difference(&described)
        .cloned()
        .collect();
    let supported_not_guarded: Vec<String> = supported
        .difference(&guarded)
        .cloned()
        .collect();

    print!("Tool description vs dispatch... ");
    if described_not_supported.is_empty() {
        println!("{}", "✅ OK".green());
    } else {
        println!("{}", "❌ Mismatch".red());
        for t in &described_not_supported {
            println!("  - described but not dispatched: {}", t);
        }
    }

    print!("Dispatch vs guard coverage...    ");
    if supported_not_guarded.is_empty() {
        println!("{}", "✅ OK".green());
    } else {
        println!("{}", "⚠️ Partial".yellow());
        for t in &supported_not_guarded {
            println!("  - dispatched but not explicitly guarded: {}", t);
        }
    }

    print!("Dispatch vs docs parity...       ");
    if supported_not_described.is_empty() {
        println!("{}", "✅ OK".green());
    } else {
        println!("{}", "⚠️ Partial".yellow());
        for t in &supported_not_described {
            println!("  - dispatched but not documented: {}", t);
        }
    }

    print!("Runtime dependency checks...     ");
    let mut dep_warnings: Vec<String> = Vec::new();

    if let Ok(cfg) = crate::config::Config::load() {
        if let Some(mcp) = cfg.mcp {
            if mcp.enabled {
                for server in mcp.servers {
                    if !command_exists_quick(&server.command) {
                        dep_warnings.push(format!(
                            "MCP server '{}' command '{}' not found",
                            server.name, server.command
                        ));
                    }
                }
            }
        }
    }

    if let Ok(workspace) = std::env::current_dir() {
        let mut loader = crate::skills::SkillLoader::new(workspace);
        if loader.scan().is_ok() {
            for skill in loader.skills().values() {
                match skill.backend.to_lowercase().as_str() {
                    "deno" => {
                        let cmd = skill
                            .deno_command
                            .as_deref()
                            .unwrap_or("deno");
                        if !command_exists_quick(cmd) {
                            dep_warnings.push(format!(
                                "Skill '{}' backend=deno requires command '{}'",
                                skill.name, cmd
                            ));
                        }
                    }
                    "mcp" => {
                        if let Some(cmd) = skill.mcp_command.as_deref()
                            && !command_exists_quick(cmd)
                        {
                            dep_warnings.push(format!(
                                "Skill '{}' backend=mcp command '{}' not found",
                                skill.name, cmd
                            ));
                        }
                    }
                    "native" => {
                        if let Some(cmd) = skill.native_command.as_deref() {
                            if !command_exists_quick(cmd) {
                                dep_warnings.push(format!(
                                    "Skill '{}' backend=native command '{}' not found",
                                    skill.name, cmd
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if dep_warnings.is_empty() {
        println!("{}", "✅ OK".green());
    } else {
        println!("{}", "⚠️ Partial".yellow());
        for w in &dep_warnings {
            println!("  - {}", w);
        }
    }

    let hard_fail = !described_not_supported.is_empty();
    println!("\nWiring Summary:");
    if hard_fail {
        println!("{}", "❌ Wiring mismatch found (described tools missing runtime dispatch).".red());
    } else {
        println!("{}", "✅ Core wiring parity checks passed.".green());
    }

    Ok(())
}

fn extract_described_tools(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();

    for line in text.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;

        while i + 1 < bytes.len() {
            if bytes[i] == b'*' && bytes[i + 1] == b'*' {
                let start = i + 2;
                let mut j = start;
                while j + 1 < bytes.len() {
                    if bytes[j] == b'*' && bytes[j + 1] == b'*' {
                        let token = line[start..j].trim();
                        if !token.is_empty()
                            && token
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                        {
                            out.insert(token.to_string());
                        }
                        i = j + 2;
                        break;
                    }
                    j += 1;
                }

                if j + 1 >= bytes.len() {
                    break;
                }
                continue;
            }
            i += 1;
        }
    }

    out
}

fn command_exists_quick(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .is_ok()
}

pub async fn run_doctor() -> Result<()> {
    println!("\n🩺 Running Flowbot Doctor...\n");

    let mut all_ok = true;

    // 1. Check Configuration
    print!("Checking Configuration... ");
    let config_result = config::Config::load();

    match &config_result {
        Ok(cfg) => {
            println!("{}", "✅ OK".green());

            // Check Provider Keys
            if let Some(ref gl) = cfg.providers.antigravity {
                 let no_key = gl.api_key.as_ref().map_or(true, |k| k.is_empty()) 
                     && gl.api_keys.as_ref().map_or(true, |ks| ks.is_empty());
                
                 if no_key {
                    println!("  ⚠️  Antigravity API key is empty.");
                    all_ok = false;
                 }
            }
            
            if let Some(ref oa) = cfg.providers.openai {
                let no_key = oa.api_key.as_ref().map_or(true, |k| k.is_empty())
                     && oa.api_keys.as_ref().map_or(true, |ks| ks.is_empty());

                if no_key {
                    // Check for tokens
                    if let Ok(tokens) = crate::config::OAuthTokens::load() {
                        if tokens.get("openai").is_some() {
                            println!("  ✅ OpenAI (OAuth token found)");
                        } else {
                            println!("  ⚠️  OpenAI key empty and no OAuth token.");
                        }
                    } else {
                        println!("  ⚠️  OpenAI API key is empty.");
                    }
                }
            }
        }
        Err(_) => {
            println!("{}", "❌ Missing or Invalid".red());
            println!("  ℹ️  Run 'flowbot setup' to create a configuration file.");
            all_ok = false;
        }
    }

    // 2. Check Database / Persistence
    print!("Checking Storage...       ");
    let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
    if let Some(parent) = db_path.parent()
        && !parent.exists()
    {
        // Try creating it to see if we have permissions
        match std::fs::create_dir_all(parent) {
            Ok(_) => {}
            Err(_) => {
                println!("{}", "❌ Permission Denied".red());
                println!("  ⚠️  Cannot create .nanobot directory.");
                all_ok = false;
            }
        }
    }

    match PersistenceManager::new(db_path.clone()).init() {
        Ok(_) => println!("{}", "✅ OK".green()),
        Err(e) => {
            println!("{}", "❌ Error".red());
            println!("  ⚠️  Database error: {}", e);
            all_ok = false;
        }
    }

    // 3. Check Internet Connectivity (Ping Google/Cloudflare)
    print!("Checking Connectivity...  ");
    match reqwest::get("https://www.google.com").await {
        Ok(_) => println!("{}", "✅ OK".green()),
        Err(_) => {
            println!("{}", "❌ Offline".red());
            println!("  ⚠️  Cannot reach the internet.");
            all_ok = false;
        }
    }

    // 4. Check Telegram
    if !telegram::check_telegram(&config_result).await {
        all_ok = false;
    }

    println!("\nSummary:");
    if all_ok {
        println!(
            "{}",
            "✨ All systems operational! You are ready to run Flowbot."
                .green()
                .bold()
        );
    } else {
        println!(
            "{}",
            "⚠️  Some issues detected. See above hints.".yellow().bold()
        );
    }

    Ok(())
}

// Simple color helper trait (if we don't want to add 'colored' dependency yet)
trait Colorize {
    fn green(self) -> String;
    fn red(self) -> String;
    fn yellow(self) -> String;
    fn bold(self) -> String;
}

impl Colorize for &str {
    fn green(self) -> String {
        format!("\x1b[32m{}\x1b[0m", self)
    }
    fn red(self) -> String {
        format!("\x1b[31m{}\x1b[0m", self)
    }
    fn yellow(self) -> String {
        format!("\x1b[33m{}\x1b[0m", self)
    }
    fn bold(self) -> String {
        format!("\x1b[1m{}\x1b[0m", self)
    }
}
