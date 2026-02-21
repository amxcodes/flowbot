use crate::config;
use crate::persistence::PersistenceManager;
use anyhow::Result;
// We'll add this for nice output, or use direct ANSI codes if keeping deps minimal
use std::collections::HashSet;
use std::path::PathBuf;

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

    let described_not_supported: Vec<String> = described.difference(&supported).cloned().collect();
    let supported_not_described: Vec<String> = supported.difference(&described).cloned().collect();
    let supported_not_guarded: Vec<String> = supported.difference(&guarded).cloned().collect();

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

    if let Ok(cfg) = crate::config::Config::load()
        && let Some(mcp) = cfg.mcp
        && mcp.enabled
    {
        for server in mcp.servers {
            if !command_exists_quick(&server.command) {
                dep_warnings.push(format!(
                    "MCP server '{}' command '{}' not found",
                    server.name, server.command
                ));
            }
        }
    }
    if let Ok(workspace) = std::env::current_dir() {
        let mut loader = crate::skills::SkillLoader::new(workspace);
        if loader.scan().is_ok() {
            for skill in loader.skills().values() {
                match skill.backend.to_lowercase().as_str() {
                    "deno" => {
                        let cmd = skill.deno_command.as_deref().unwrap_or("deno");
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
                        if let Some(cmd) = skill.native_command.as_deref()
                            && !command_exists_quick(cmd)
                        {
                            dep_warnings.push(format!(
                                "Skill '{}' backend=native command '{}' not found",
                                skill.name, cmd
                            ));
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
        println!(
            "{}",
            "❌ Wiring mismatch found (described tools missing runtime dispatch).".red()
        );
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
                            && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
    if std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .is_ok()
    {
        return true;
    }

    if cfg!(windows) {
        if let Ok(out) = std::process::Command::new("where").arg(cmd).output()
            && out.status.success()
            && !out.stdout.is_empty()
        {
            return true;
        }

        if cmd.eq_ignore_ascii_case("gh")
            && std::path::Path::new("C:\\Program Files\\GitHub CLI\\gh.exe").exists()
        {
            return true;
        }
    }

    false
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
                let no_key = gl.api_key.as_ref().is_none_or(|k| k.is_empty())
                    && gl.api_keys.as_ref().is_none_or(|ks| ks.is_empty());

                if no_key {
                    println!("  ⚠️  Antigravity API key is empty.");
                    all_ok = false;
                }
            }

            if let Some(ref oa) = cfg.providers.openai {
                let no_key = oa.api_key.as_ref().is_none_or(|k| k.is_empty())
                    && oa.api_keys.as_ref().is_none_or(|ks| ks.is_empty());

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
            println!("  ℹ️  Run 'nanobot setup' to create a configuration file.");
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

    // 5. Common runtime dependencies for community skills
    print!("Checking Skill Runtimes... ");
    let mut runtime_issues = Vec::new();
    for cmd in ["gh", "deno"] {
        if !command_exists_quick(cmd) {
            runtime_issues.push(format!("missing command: {}", cmd));
        }
    }
    if !command_exists_quick("gog") {
        runtime_issues
            .push("optional command missing: gog (needed for Google Workspace skill)".to_string());
    }
    if !command_exists_quick("node") {
        runtime_issues.push("optional command missing: node".to_string());
    } else if let Some(major) = command_major_version("node") {
        if major < 22 {
            runtime_issues.push(format!(
                "node version too old: {} (recommended >= 22 for legacy skills)",
                major
            ));
        }
    } else {
        runtime_issues.push("could not parse node version".to_string());
    }

    if runtime_issues.is_empty() {
        println!("{}", "✅ OK".green());
    } else {
        println!("{}", "⚠️ Partial".yellow());
        for issue in &runtime_issues {
            println!("  - {}", issue);
        }
        if runtime_issues
            .iter()
            .any(|i| i.starts_with("missing command"))
        {
            all_ok = false;
        }
    }

    // 6. OpenClaw auth bridge path permissions
    print!("Checking Auth Bridge...    ");
    match check_openclaw_auth_writable() {
        Ok(_) => println!("{}", "✅ OK".green()),
        Err(e) => {
            println!("{}", "❌ Error".red());
            println!("  ⚠️  Cannot write ~/.openclaw/auth: {}", e);
            all_ok = false;
        }
    }

    // 7. MCP configuration readiness (recommended)
    print!("Checking MCP Registry...   ");
    match &config_result {
        Ok(cfg) => {
            if let Some(mcp) = cfg.mcp.as_ref() {
                if !mcp.enabled {
                    println!("{}", "⚪ Disabled".yellow());
                } else {
                    let mut missing = Vec::new();
                    for server in &mcp.servers {
                        if !command_exists_quick(&server.command) {
                            missing.push(format!("{} ({})", server.name, server.command));
                        }
                    }

                    if missing.is_empty() {
                        println!("{}", "✅ OK".green());
                    } else {
                        println!("{}", "⚠️ Partial".yellow());
                        for item in missing {
                            println!("  - missing MCP server command: {}", item);
                        }
                    }
                }
            } else {
                println!("{}", "⚪ Not Configured".yellow());
            }
        }
        Err(_) => println!("{}", "⚪ Skipped".yellow()),
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

fn check_openclaw_auth_writable() -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Home directory not found"))?;
    let auth_dir = home.join(".openclaw").join("auth");
    std::fs::create_dir_all(&auth_dir)?;

    let probe = auth_dir.join(".nanobot_write_probe");
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(&probe)?;
    Ok(())
}

fn command_major_version(cmd: &str) -> Option<u64> {
    let output = std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let trimmed = raw.trim();
    let normalized = trimmed.strip_prefix('v').unwrap_or(trimmed);
    normalized.split('.').next()?.parse::<u64>().ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn described_tools_have_runtime_support() {
        let described = extract_described_tools(&crate::tools::executor::get_tool_descriptions());
        let supported: std::collections::HashSet<String> =
            crate::tools::executor::supported_tool_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();

        let missing: Vec<String> = described.difference(&supported).cloned().collect();
        assert!(
            missing.is_empty(),
            "described tools missing from runtime support: {:?}",
            missing
        );
    }

    #[test]
    fn supported_tools_have_guard_coverage() {
        let supported: std::collections::HashSet<String> =
            crate::tools::executor::supported_tool_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();
        let guarded: std::collections::HashSet<String> =
            crate::tools::guard::ToolGuard::guarded_tool_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();

        let missing: Vec<String> = supported.difference(&guarded).cloned().collect();
        assert!(
            missing.is_empty(),
            "supported tools missing guard coverage: {:?}",
            missing
        );
    }
}
