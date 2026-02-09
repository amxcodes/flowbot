use console::style;

/// Print a formatted instruction box similar to OpenClaw's style
pub fn print_instruction_box(title: &str, lines: &[&str]) {
    println!();
    println!("{}", style(format!("◇  {} ", title)).bold());
    println!("{}", style("│").dim());
    
    for line in lines {
        println!("{} {}", style("│").dim(), line);
    }
    
    println!("{}", style("│").dim());
    println!("{}", style("├─────────────────────────────────────────────────────────────╯").dim());
    println!();
}

/// Print pairing system explanation
pub fn print_pairing_explanation() {
    print_instruction_box(
        "How DM Security Works",
        &[
            "By default, Nanobot uses pairing for security.",
            "",
            "When someone DMs your bot for the first time, they receive a pairing code.",
            "Approve the pairing with:",
            "  nanobot pairing approve <channel> <code>",
            "",
            "Example: nanobot pairing approve telegram A1B2C3",
            "",
            "For open DMs (not recommended), set dmPolicy=\"open\" in config.",
        ],
    );
}

/// Print channel status table
pub fn print_channel_status(channels: &[(String, String)]) {
    println!();
    println!("{}", style("◇ Channel Status").bold());
    println!("{}", style("│").dim());
    
    if channels.is_empty() {
        println!("{} {}", style("│").dim(), style("No channels configured").yellow());
    } else {
        for (name, status) in channels {
            println!("{} {}: {}", style("│").dim(), name, status);
        }
    }
    
    println!("{}", style("│").dim());
    println!("{}", style("├─────────────────────────────────────").dim());
    println!();
}

/// Print gateway launch instructions
pub fn print_gateway_instructions() {
    print_instruction_box(
        "Gateway Service",
        &[
            "The gateway connects all your messaging channels.",
            "",
            "To start manually:",
            "  nanobot gateway",
            "",
            "To run as systemd service:",
            "  nanobot service start",
            "",
            "Check status:",
            "  nanobot service status",
        ],
    );
}
