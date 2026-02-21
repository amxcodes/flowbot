use super::confirmation::{
    ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse, RiskLevel,
};
use anyhow::Result;
use async_trait::async_trait;
use colored::Colorize;
use std::io::{self, IsTerminal, Write};

/// CLI confirmation adapter using terminal prompts
pub struct CliConfirmationAdapter;

impl CliConfirmationAdapter {
    pub fn new() -> Self {
        Self
    }

    fn format_risk_emoji(risk: RiskLevel) -> &'static str {
        match risk {
            RiskLevel::Low => "ℹ️",
            RiskLevel::Medium => "⚠️",
            RiskLevel::High => "🚨",
            RiskLevel::Critical => "💀",
        }
    }

    fn format_risk_text(risk: RiskLevel) -> colored::ColoredString {
        match risk {
            RiskLevel::Low => "LOW".green(),
            RiskLevel::Medium => "MEDIUM".yellow(),
            RiskLevel::High => "HIGH".red(),
            RiskLevel::Critical => "CRITICAL".red().bold(),
        }
    }
}

#[async_trait]
impl ConfirmationAdapter for CliConfirmationAdapter {
    async fn request_confirmation(
        &self,
        request: &ConfirmationRequest,
    ) -> Result<ConfirmationResponse> {
        // Print header
        println!();
        println!(
            "{}",
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_cyan()
        );
        println!(
            "{} {} Permission Request",
            Self::format_risk_emoji(request.risk_level),
            "SECURITY:".bright_cyan().bold()
        );
        println!(
            "{}",
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_cyan()
        );

        // Print details
        println!(
            "{}: {}",
            "Tool".bright_white().bold(),
            request.tool_name.cyan()
        );
        println!(
            "{}: {}",
            "Operation".bright_white().bold(),
            request.operation
        );
        println!(
            "{}: {}",
            "Arguments".bright_white().bold(),
            request.args.yellow()
        );
        println!(
            "{}: {}",
            "Risk Level".bright_white().bold(),
            Self::format_risk_text(request.risk_level)
        );

        // Print warning for critical operations
        if matches!(request.risk_level, RiskLevel::Critical) {
            println!();
            println!(
                "{} {}",
                "⚠️".red(),
                "This operation could potentially damage your system!"
                    .red()
                    .bold()
            );
        }

        println!(
            "{}",
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_cyan()
        );

        // Prompt for decision
        print!(
            "{} {} ",
            "Allow this operation?".bright_white().bold(),
            "[y/n/always]:".bright_white()
        );
        io::stdout().flush()?;

        // Read user input
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();

        let (allowed, remember) = match input.as_str() {
            "y" | "yes" => (true, false),
            "n" | "no" => (false, false),
            "always" | "a" => (true, true),
            _ => {
                println!("{}", "Invalid input, defaulting to DENY".red());
                (false, false)
            }
        };

        if allowed {
            println!("{}", "✓ Operation ALLOWED".green().bold());
        } else {
            println!("{}", "✗ Operation DENIED".red().bold());
        }
        println!();

        Ok(ConfirmationResponse {
            id: request.id.clone(),
            allowed,
            remember,
        })
    }

    fn name(&self) -> &str {
        "CLI"
    }

    async fn is_available(&self) -> bool {
        io::stdin().is_terminal()
    }
}

impl Default for CliConfirmationAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_adapter_creation() {
        let adapter = CliConfirmationAdapter::new();
        assert_eq!(adapter.name(), "CLI");
    }

    #[test]
    fn test_risk_formatting() {
        assert_eq!(
            CliConfirmationAdapter::format_risk_emoji(RiskLevel::Low),
            "ℹ️"
        );
        assert_eq!(
            CliConfirmationAdapter::format_risk_emoji(RiskLevel::Critical),
            "💀"
        );
    }
}
