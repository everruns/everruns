// TUI app state and event loop.
// Decision: keep the rendering layer tiny — a scrolling chat above a single
// input box. The event loop multiplexes terminal keystrokes with a background
// turn so the UI stays responsive while the model is thinking.

use crate::runtime::RuntimeBundle;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use everruns_core::events::EventData;
use everruns_core::message::MessageRole;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub enum Author {
    User,
    Assistant,
    Tool,
    System,
}

impl Author {
    fn label(&self) -> &'static str {
        match self {
            Author::User => "you",
            Author::Assistant => "agent",
            Author::Tool => "tool",
            Author::System => "system",
        }
    }
    fn color(&self) -> Color {
        match self {
            Author::User => Color::LightCyan,
            Author::Assistant => Color::LightGreen,
            Author::Tool => Color::Yellow,
            Author::System => Color::DarkGray,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub author: Author,
    pub text: String,
}

pub struct App {
    bundle: Arc<RuntimeBundle>,
    pub lines: Vec<ChatLine>,
    pub input: String,
    pub busy: bool,
    pub should_quit: bool,
    pub scroll: u16,
    rx: Option<mpsc::UnboundedReceiver<TurnEvent>>,
}

#[derive(Debug)]
enum TurnEvent {
    Lines(Vec<ChatLine>),
    Done,
    Failed(String),
}

impl App {
    pub fn new(bundle: Arc<RuntimeBundle>) -> Self {
        let mut app = Self {
            bundle,
            lines: Vec::new(),
            input: String::new(),
            busy: false,
            should_quit: false,
            scroll: 0,
            rx: None,
        };
        app.emit_system_banner();
        app
    }

    fn emit_system_banner(&mut self) {
        self.push_system(format!(
            "workspace: {}",
            self.bundle.workspace_root.display()
        ));
        self.push_system(format!("model: {}", self.bundle.provider_label));
        self.push_system(format!("tools: {}", self.bundle.tool_names.join(", ")));
        if !self.bundle.instruction_summary.is_empty() {
            self.push_system(format!(
                "instructions: {}",
                self.bundle.instruction_summary.join(", ")
            ));
        }
        self.push_system("type /help for commands, Esc or Ctrl-D to exit".into());
    }

    fn push_user(&mut self, text: String) {
        self.lines.push(ChatLine {
            author: Author::User,
            text,
        });
    }
    fn push_system(&mut self, text: String) {
        self.lines.push(ChatLine {
            author: Author::System,
            text,
        });
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| draw(f, self))?;

            if let Some(rx) = self.rx.as_mut() {
                match rx.try_recv() {
                    Ok(TurnEvent::Lines(lines)) => {
                        self.lines.extend(lines);
                        continue;
                    }
                    Ok(TurnEvent::Done) => {
                        self.busy = false;
                        self.rx = None;
                        continue;
                    }
                    Ok(TurnEvent::Failed(err)) => {
                        self.busy = false;
                        self.rx = None;
                        self.push_system(format!("turn failed: {err}"));
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        self.busy = false;
                        self.rx = None;
                    }
                }
            }

            if event::poll(Duration::from_millis(80))?
                && let Event::Key(key) = event::read()?
            {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                self.handle_key(key).await;
            }
            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
            self.should_quit = true;
            return;
        }
        if self.busy {
            // Ignore input while a turn is running.
            return;
        }
        match key.code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.input);
                let text = text.trim().to_string();
                if text.is_empty() {
                    return;
                }
                if let Some(rest) = text.strip_prefix('/') {
                    self.handle_command(rest);
                    return;
                }
                self.push_user(text.clone());
                self.start_turn(text);
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(5);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(5);
            }
            _ => {}
        }
    }

    fn handle_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        let head = parts[0];
        match head {
            "help" => {
                self.push_system(
                    "/help · /tools · /cwd · /model · /clear · /quit (Esc/Ctrl-D also exit)".into(),
                );
            }
            "tools" => {
                self.push_system(format!("tools: {}", self.bundle.tool_names.join(", ")));
            }
            "cwd" => {
                self.push_system(format!(
                    "workspace root: {}",
                    self.bundle.workspace_root.display()
                ));
            }
            "model" => {
                self.push_system(format!("model: {}", self.bundle.provider_label));
            }
            "clear" => {
                self.lines.clear();
                self.emit_system_banner();
            }
            "quit" | "exit" => self.should_quit = true,
            other => self.push_system(format!("unknown command: /{other}")),
        }
    }

    fn start_turn(&mut self, prompt: String) {
        let bundle = self.bundle.clone();
        let (tx, rx) = mpsc::unbounded_channel::<TurnEvent>();
        self.rx = Some(rx);
        self.busy = true;

        tokio::spawn(async move {
            let session_id = bundle.session_id;
            let before = match bundle.runtime.messages(session_id).await {
                Ok(m) => m.len(),
                Err(e) => {
                    let _ = tx.send(TurnEvent::Failed(format!("load history: {e}")));
                    let _ = tx.send(TurnEvent::Done);
                    return;
                }
            };
            let events_before = match bundle.runtime.events().await {
                Ok(e) => e.len(),
                Err(_) => 0,
            };

            let result = bundle.runtime.run_text_turn(session_id, prompt).await;
            let response = match result {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(TurnEvent::Failed(format!("{e}")));
                    let _ = tx.send(TurnEvent::Done);
                    return;
                }
            };

            let messages = bundle
                .runtime
                .messages(session_id)
                .await
                .unwrap_or_default();
            let events = bundle.runtime.events().await.unwrap_or_default();

            let mut out = Vec::new();
            // Tool calls executed during this turn surface as new events.
            for event in events.iter().skip(events_before) {
                if let EventData::ToolCompleted(data) = &event.data {
                    let marker = if data.success { "✓" } else { "✗" };
                    out.push(ChatLine {
                        author: Author::Tool,
                        text: format!("{} {marker}", data.tool_name),
                    });
                }
            }
            // Assistant text from the turn.
            for msg in messages.iter().skip(before) {
                if msg.role == MessageRole::Agent
                    && let Some(text) = msg.text()
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(ChatLine {
                            author: Author::Assistant,
                            text: trimmed.to_string(),
                        });
                    }
                }
            }
            if out.is_empty() && !response.response.is_empty() {
                out.push(ChatLine {
                    author: Author::Assistant,
                    text: response.response,
                });
            }
            if !response.success
                && let Some(err) = response.error
            {
                out.push(ChatLine {
                    author: Author::System,
                    text: format!("turn error: {err}"),
                });
            }
            let _ = tx.send(TurnEvent::Lines(out));
            let _ = tx.send(TurnEvent::Done);
        });
    }
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_chat(f, chunks[0], app);
    draw_input(f, chunks[1], app);
    draw_status(f, chunks[2], app);
}

fn draw_chat(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let inner_width = area.width.saturating_sub(2).max(10) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for chat in &app.lines {
        let header = Span::styled(
            format!("{} › ", chat.author.label()),
            Style::default()
                .fg(chat.author.color())
                .add_modifier(Modifier::BOLD),
        );
        let wrapped = textwrap::wrap(&chat.text, inner_width.saturating_sub(8).max(20));
        let mut first = true;
        for piece in wrapped {
            if first {
                lines.push(Line::from(vec![
                    header.clone(),
                    Span::raw(piece.into_owned()),
                ]));
                first = false;
            } else {
                lines.push(Line::from(vec![
                    Span::raw("        "),
                    Span::raw(piece.into_owned()),
                ]));
            }
        }
        lines.push(Line::from(""));
    }
    let height = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    let max_scroll = total.saturating_sub(height);
    let scroll_back = (app.scroll as usize).min(max_scroll);
    let end = total.saturating_sub(scroll_back);
    let start = end.saturating_sub(height);
    let view: Vec<Line> = lines[start..end].to_vec();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Everruns Coding CLI ");
    let para = Paragraph::new(view).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn draw_input(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let title = if app.busy {
        " thinking… (input disabled) "
    } else {
        " message (Enter to send) "
    };
    let para = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
    if !app.busy {
        let x = area.x + 1 + (app.input.len() as u16).min(area.width.saturating_sub(2));
        let y = area.y + 1;
        f.set_cursor_position((x, y));
    }
}

fn draw_status(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let status = format!(
        " {} · {} · {}msgs ",
        app.bundle.provider_label,
        app.bundle.workspace_root.display(),
        app.lines.len()
    );
    let para = Paragraph::new(Span::styled(status, Style::default().fg(Color::DarkGray)));
    f.render_widget(para, area);
}
