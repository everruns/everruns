// TUI app state and event loop.
// Decision: keep the rendering layer tiny — a scrolling chat above a single
// input box, plus a modal approval bar for destructive tools. The event loop
// multiplexes terminal keystrokes with a background turn task and an approval
// channel so the UI stays responsive while the model is thinking.

use crate::approval::ApprovalRequest;
use crate::runtime::RuntimeBundle;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use everruns_core::events::{EventData, ToolCompletedData};
use everruns_core::message::{ContentPart, MessageRole};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug)]
pub enum Author {
    User,
    Assistant,
    Tool,
    Diff,
    System,
}

impl Author {
    fn label(&self) -> &'static str {
        match self {
            Author::User => "you",
            Author::Assistant => "agent",
            Author::Tool => "tool",
            Author::Diff => "diff",
            Author::System => "system",
        }
    }
    fn color(&self) -> Color {
        match self {
            Author::User => Color::LightCyan,
            Author::Assistant => Color::LightGreen,
            Author::Tool => Color::Yellow,
            Author::Diff => Color::Magenta,
            Author::System => Color::DarkGray,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub author: Author,
    pub text: String,
}

type ApprovalRx = mpsc::UnboundedReceiver<(ApprovalRequest, oneshot::Sender<bool>)>;

struct PendingApproval {
    req: ApprovalRequest,
    responder: oneshot::Sender<bool>,
}

pub struct App {
    bundle: Arc<RuntimeBundle>,
    pub lines: Vec<ChatLine>,
    pub input: String,
    pub busy: bool,
    pub should_quit: bool,
    pub scroll: u16,
    rx: Option<mpsc::UnboundedReceiver<TurnEvent>>,
    approval_rx: ApprovalRx,
    pending: Option<PendingApproval>,
}

#[derive(Debug)]
enum TurnEvent {
    Lines(Vec<ChatLine>),
    Done,
    Failed(String),
}

impl App {
    pub fn new(bundle: Arc<RuntimeBundle>, approval_rx: ApprovalRx) -> Self {
        let mut app = Self {
            bundle,
            lines: Vec::new(),
            input: String::new(),
            busy: false,
            should_quit: false,
            scroll: 0,
            rx: None,
            approval_rx,
            pending: None,
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
        self.push_system("type /help for commands, Esc or Ctrl-D to exit; approvals: y / n".into());
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

            // 1) drain background turn events
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

            // 2) drain pending approval requests
            if self.pending.is_none()
                && let Ok((req, responder)) = self.approval_rx.try_recv()
            {
                let header = format!("approval needed: {}", req.headline());
                self.push_system(header);
                let detail = req.detail();
                for line in detail.lines().take(40) {
                    self.lines.push(ChatLine {
                        author: Author::Diff,
                        text: line.to_string(),
                    });
                }
                self.pending = Some(PendingApproval { req, responder });
            }

            // 3) keystrokes
            if event::poll(Duration::from_millis(80))?
                && let Event::Key(key) = event::read()?
            {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                self.handle_key(key).await;
            }
            if self.should_quit {
                // If we exit with an outstanding approval, deny it so the tool
                // task unblocks and the runtime can record a tool error.
                if let Some(p) = self.pending.take() {
                    let _ = p.responder.send(false);
                }
                break;
            }
        }
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('d'))
        {
            self.should_quit = true;
            return;
        }

        // Approval mode: only y / n / Esc.
        if self.pending.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(p) = self.pending.take() {
                        let _ = p.responder.send(true);
                        self.push_system("approved".into());
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    if let Some(p) = self.pending.take() {
                        let _ = p.responder.send(false);
                        self.push_system("denied".into());
                    }
                }
                _ => {}
            }
            return;
        }

        if self.busy {
            // Ignore input while a turn is running (other than approvals above).
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
                    let summary = summarize_tool_result(data);
                    let line = if summary.is_empty() {
                        format!("{} {marker}", data.tool_name)
                    } else {
                        format!("{} {marker}  {summary}", data.tool_name)
                    };
                    out.push(ChatLine {
                        author: Author::Tool,
                        text: line,
                    });
                    // For edits, also surface the diff inline so the operator
                    // can see exactly what changed.
                    if data.tool_name == "edit_file"
                        && let Some(diff) = extract_field(data, "diff")
                    {
                        for line in diff.lines().take(40) {
                            out.push(ChatLine {
                                author: Author::Diff,
                                text: line.to_string(),
                            });
                        }
                    }
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

// ---------- helpers for surfacing tool results ----------

fn result_value(data: &ToolCompletedData) -> Option<Value> {
    let parts = data.result.as_ref()?;
    for part in parts {
        if let ContentPart::Text(t) = part
            && let Ok(v) = serde_json::from_str::<Value>(&t.text)
        {
            return Some(v);
        }
    }
    None
}

fn extract_field(data: &ToolCompletedData, field: &str) -> Option<String> {
    let v = result_value(data)?;
    v.get(field).and_then(|s| s.as_str()).map(str::to_string)
}

/// One-line summary of a tool result, used in the transcript and `--print` output.
pub fn summarize_tool_result(data: &ToolCompletedData) -> String {
    let Some(v) = result_value(data) else {
        if let Some(err) = &data.error {
            return format!("error: {}", first_line(err, 120));
        }
        return String::new();
    };
    match data.tool_name.as_str() {
        "read_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let lines = v.get("lines_returned").and_then(Value::as_u64).unwrap_or(0);
            let total = v.get("total_lines").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({lines}/{total} lines)")
        }
        "write_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let bytes = v.get("bytes_written").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({bytes} bytes)")
        }
        "edit_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let n = v.get("replacements").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({n} replacement(s))")
        }
        "list_directory" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let n = v
                .get("entries")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            format!("{path} ({n} entries)")
        }
        "grep" => {
            let pattern = v.get("pattern").and_then(Value::as_str).unwrap_or("");
            let n = v.get("match_count").and_then(Value::as_u64).unwrap_or(0);
            format!("/{pattern}/ ({n} match(es))")
        }
        "bash" => {
            let cmd = v
                .get("command")
                .and_then(Value::as_str)
                .map(|c| first_line(c, 80))
                .unwrap_or_default();
            let code = v
                .get("exit_code")
                .and_then(Value::as_i64)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into());
            format!("`{cmd}` exit={code}")
        }
        _ => String::new(),
    }
}

fn first_line(s: &str, max: usize) -> String {
    let l = s.lines().next().unwrap_or("");
    if l.len() > max {
        format!("{}…", &l[..max])
    } else {
        l.to_string()
    }
}

// ---------- rendering ----------

fn draw(f: &mut ratatui::Frame, app: &App) {
    let has_approval = app.pending.is_some();
    let input_height: u16 = 3;
    let approval_height: u16 = if has_approval { 3 } else { 0 };

    let mut constraints = vec![Constraint::Min(3)];
    if has_approval {
        constraints.push(Constraint::Length(approval_height));
    }
    constraints.push(Constraint::Length(input_height));
    constraints.push(Constraint::Length(1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    let mut idx = 0;
    draw_chat(f, chunks[idx], app);
    idx += 1;
    if has_approval {
        draw_approval(f, chunks[idx], app);
        idx += 1;
    }
    draw_input(f, chunks[idx], app);
    idx += 1;
    draw_status(f, chunks[idx], app);
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

fn draw_approval(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let title = " approve? press y to allow, n / Esc to deny ";
    let text = app
        .pending
        .as_ref()
        .map(|p| p.req.headline())
        .unwrap_or_default();
    let para = Paragraph::new(Span::styled(
        text,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(para, area);
}

fn draw_input(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let title = if app.pending.is_some() {
        " approval pending — answer y / n above "
    } else if app.busy {
        " thinking… (input disabled) "
    } else {
        " message (Enter to send) "
    };
    let para = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
    if !app.busy && app.pending.is_none() {
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
