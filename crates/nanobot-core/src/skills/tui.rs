// Standalone Skills TUI - Interactive visual skills manager
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::io;

use crate::skills::{
    SkillLoader,
    config::{SkillSetupWizard, SkillsConfig},
};

#[derive(Clone)]
struct SkillInfo {
    name: String,
    enabled: bool,
    has_credentials: bool,
    tool_count: usize,
    category: String,
}

pub struct SkillsTUI {
    skills: Vec<SkillInfo>,
    selected_index: usize,
    status_message: String,
}

impl SkillsTUI {
    pub fn new() -> Result<Self> {
        Ok(Self {
            skills: Vec::new(),
            selected_index: 0,
            status_message: String::new(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        // Load skills
        self.load_skills()?;

        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.run_app(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    fn load_skills(&mut self) -> Result<()> {
        let workspace_dir = std::env::current_dir()?;
        let mut loader = SkillLoader::new(workspace_dir);
        loader.scan()?;

        let config = SkillsConfig::load().unwrap_or_default();

        self.skills = loader
            .skills()
            .values()
            .map(|skill| {
                let enabled = config.is_enabled(&skill.name);
                let has_credentials = config.get_credential(&skill.name, "api_key").is_some()
                    || config.get_credential(&skill.name, "client_id").is_some();

                SkillInfo {
                    name: skill.name.clone(),
                    enabled,
                    has_credentials,
                    tool_count: skill.tools.len(),
                    category: skill.category.clone(),
                }
            })
            .collect();

        Ok(())
    }

    fn run_app(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3), // Header
                        Constraint::Min(1),    // Skills list
                        Constraint::Length(3), // Status/Help
                    ])
                    .split(f.area());

                // Header
                let header = Paragraph::new("🚀 Nanobot Skills Manager")
                    .style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(header, chunks[0]);

                // Skills list
                let skills: Vec<ListItem> = self
                    .skills
                    .iter()
                    .enumerate()
                    .map(|(i, skill)| {
                        let status_icon = if skill.enabled { "✓" } else { "✗" };
                        let status_color = if skill.enabled {
                            Color::Green
                        } else {
                            Color::Red
                        };
                        let cred_icon = if skill.has_credentials { "🔑" } else { "  " };

                        let content = Line::from(vec![
                            Span::raw(if i == self.selected_index { "> " } else { "  " }),
                            Span::styled(status_icon, Style::default().fg(status_color)),
                            Span::raw(" "),
                            Span::styled(
                                format!("{:<15}", skill.name),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(cred_icon),
                            Span::raw(format!(" {:>2} tools  ", skill.tool_count)),
                            Span::styled(&skill.category, Style::default().fg(Color::DarkGray)),
                        ]);

                        ListItem::new(content).style(if i == self.selected_index {
                            Style::default().bg(Color::DarkGray)
                        } else {
                            Style::default()
                        })
                    })
                    .collect();

                let skills_list =
                    List::new(skills).block(Block::default().borders(Borders::ALL).title(
                        "Skills (↑↓:Navigate  e:Enable  d:Disable  s:Setup  r:Refresh  q:Quit)",
                    ));
                f.render_widget(skills_list, chunks[1]);

                // Status message
                let status = Paragraph::new(self.status_message.as_str())
                    .style(Style::default().fg(Color::Yellow))
                    .block(Block::default().borders(Borders::ALL).title("Status"));
                f.render_widget(status, chunks[2]);
            })?;

            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        break;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.selected_index > 0 {
                            self.selected_index -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.selected_index < self.skills.len().saturating_sub(1) {
                            self.selected_index += 1;
                        }
                    }
                    KeyCode::Char('e') => {
                        self.enable_skill()?;
                    }
                    KeyCode::Char('d') => {
                        self.disable_skill()?;
                    }
                    KeyCode::Char('s') => {
                        self.setup_skill()?;
                    }
                    KeyCode::Char('r') => {
                        self.load_skills()?;
                        self.status_message = "✓ Skills refreshed".to_string();
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn enable_skill(&mut self) -> Result<()> {
        if let Some(skill) = self.skills.get_mut(self.selected_index) {
            let mut config = SkillsConfig::load().unwrap_or_default();
            config.enable_skill(&skill.name);
            config.save()?;
            skill.enabled = true;
            self.status_message = format!("✓ Enabled skill: {}", skill.name);
        }
        Ok(())
    }

    fn disable_skill(&mut self) -> Result<()> {
        if let Some(skill) = self.skills.get_mut(self.selected_index) {
            let mut config = SkillsConfig::load().unwrap_or_default();
            config.disable_skill(&skill.name);
            config.save()?;
            skill.enabled = false;
            self.status_message = format!("✗ Disabled skill: {}", skill.name);
        }
        Ok(())
    }

    fn setup_skill(&mut self) -> Result<()> {
        // Clone skill name to avoid borrow conflict with load_skills() later
        let skill_name_opt = self.skills.get(self.selected_index).map(|s| s.name.clone());

        if let Some(skill_name) = skill_name_opt {
            // Exit TUI temporarily
            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

            // Run setup wizard
            let mut wizard = SkillSetupWizard::new()?;
            wizard.setup_skill(&skill_name)?;
            wizard.save()?;

            // Re-enter TUI
            enable_raw_mode()?;
            execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;

            // Reload skills to update state
            self.load_skills()?;
            self.status_message = format!("✓ Setup complete: {}", skill_name);
        }
        Ok(())
    }
}
