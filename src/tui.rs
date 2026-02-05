use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use std::io;

pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub struct ChatUI {
    messages: Vec<ChatMessage>,
    input: String,
    provider: String,
}

impl ChatUI {
    pub fn new(provider: String) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            provider,
        }
    }

    pub fn add_message(&mut self, role: String, content: String) {
        self.messages.push(ChatMessage { role, content });
    }

    pub fn run(&mut self) -> io::Result<Option<String>> {
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

    fn run_app(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<Option<String>> {
        loop {
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),  // Header
                        Constraint::Min(1),     // Messages
                        Constraint::Length(3),  // Input
                    ])
                    .split(f.area());

                // Header
                let header = Paragraph::new(format!("Nanobot [Provider: {}] | Press Esc to quit", self.provider))
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(header, chunks[0]);

                // Messages
                let messages: Vec<ListItem> = self
                    .messages
                    .iter()
                    .map(|m| {
                        let style = if m.role == "user" {
                            Style::default().fg(Color::Green)
                        } else {
                            Style::default().fg(Color::Yellow)
                        };
                        
                        let content = Line::from(vec![
                            Span::styled(format!("{}: ", m.role), style.add_modifier(Modifier::BOLD)),
                            Span::raw(&m.content),
                        ]);
                        
                        ListItem::new(content)
                    })
                    .collect();

                let messages_list = List::new(messages)
                    .block(Block::default().borders(Borders::ALL).title("Chat"));
                f.render_widget(messages_list, chunks[1]);

                // Input
                let input = Paragraph::new(self.input.as_str())
                    .style(Style::default().fg(Color::White))
                    .block(Block::default().borders(Borders::ALL).title("Input (Enter to send)"));
                f.render_widget(input, chunks[2]);
            })?;

            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => {
                        return Ok(None);
                    }
                    KeyCode::Enter => {
                        if !self.input.is_empty() {
                            let message = self.input.clone();
                            self.input.clear();
                            return Ok(Some(message));
                        }
                    }
                    KeyCode::Char(c) => {
                        self.input.push(c);
                    }
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    _ => {}
                }
            }
        }
    }
}
