use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
};
use std::{
    io,
    time::Duration,
};

pub struct TuiManager {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    input_buffer: String,
    input_history: Vec<String>,
    history_index: usize,
}

impl TuiManager {
    pub fn new() -> Result<Self> {
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            input_buffer: String::new(),
            input_history: Vec::new(),
            history_index: 0,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;
        self.terminal.clear()?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    pub fn draw(&mut self) -> Result<()> {
        self.terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Header
                    Constraint::Min(0),    // Logs
                    Constraint::Length(3), // Status
                    Constraint::Length(3), // Input
                ])
                .split(f.area());

            // Header with branding
            let header = Paragraph::new(" NANOBOT v0.1.0 ")
                .style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            f.render_widget(header, chunks[0]);

            // Logs window
            let tui_widget = tui_logger::TuiLoggerWidget::default()
                .style_error(Style::default().fg(Color::Red))
                .style_warn(Style::default().fg(Color::Yellow))
                .style_info(Style::default().fg(Color::Blue))
                .block(Block::default().borders(Borders::ALL).title("Logs"));
            f.render_widget(tui_widget, chunks[1]);

            // Status
            let status = Paragraph::new(" Ready ")
                .block(Block::default().borders(Borders::ALL).title("Status"));
            f.render_widget(status, chunks[2]);

            // Input
            let input = Paragraph::new(self.input_buffer.as_str())
                .block(Block::default().borders(Borders::ALL).title("Input"));
            f.render_widget(input, chunks[3]);
        })?;
        Ok(())
    }

    pub async fn wait_for_input(&mut self) -> Result<Option<String>> {
        loop {
            self.draw()?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(_key) = event::read()? {
                    if let Some(result) = self.handle_event(Event::Key(_key))? {
                        return Ok(Some(result));
                    }
                }
            }
        }
    }

    /// Handle user input events
    pub fn handle_event(&mut self, event: Event) -> Result<Option<String>> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    return Ok(Some("/quit".to_string()));
                }
                KeyCode::Esc => {
                    return Ok(Some("/quit".to_string()));
                }
                KeyCode::Up => {
                    if !self.input_history.is_empty() {
                        if self.history_index > 0 {
                            self.history_index -= 1;
                            self.input_buffer = self.input_history[self.history_index].clone();
                        }
                    }
                }
                KeyCode::Down => {
                    if !self.input_history.is_empty() {
                        if self.history_index < self.input_history.len() {
                            self.history_index += 1;
                            if self.history_index == self.input_history.len() {
                                self.input_buffer.clear();
                            } else {
                                self.input_buffer = self.input_history[self.history_index].clone();
                            }
                        }
                    }
                }
                KeyCode::Char(c) => {
                    self.input_buffer.push(c);
                }
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                }
                KeyCode::Enter => {
                    let input = std::mem::take(&mut self.input_buffer);
                    if !input.trim().is_empty() {
                        self.input_history.push(input.clone());
                        self.history_index = self.input_history.len();

                        // Check for local shell command (bang command)
                        if input.trim().starts_with("!") {
                            let cmd_line = input.trim().trim_start_matches('!').trim();
                            if !cmd_line.is_empty() {
                                // Execute command
                                // Note: This bypasses the agent loop and SafeTool policy for now
                                // In a real implementation this should use SafeTool
                                match std::process::Command::new("cmd")
                                    .args(["/C", cmd_line])
                                    .output()
                                {
                                    Ok(output) => {
                                        let stdout = String::from_utf8_lossy(&output.stdout);
                                        let stderr = String::from_utf8_lossy(&output.stderr);
                                        if !stdout.is_empty() {
                                            log::info!("$ {}\n{}", cmd_line, stdout.trim());
                                        }
                                        if !stderr.is_empty() {
                                            log::error!("$ {}\n{}", cmd_line, stderr.trim());
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to execute '{}': {}", cmd_line, e);
                                    }
                                }
                                // Return None so the agent doesn't see this as a prompt
                                return Ok(None);
                            }
                        }
                    }
                    return Ok(Some(input));
                }
                _ => {}
            }
        }
        Ok(None)
    }
}
