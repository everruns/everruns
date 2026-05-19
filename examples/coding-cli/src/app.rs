// TUI app state and event loop.
// Decision: keep the rendering layer tiny — a scrolling chat above a single
// input box, plus a modal approval bar for destructive tools. The event loop
// multiplexes terminal keystrokes with a background turn task and an approval
// channel so the UI stays responsive while the model is thinking.

use crate::approval::ApprovalRequest;
use crate::runtime::RuntimeBundle;
use anyhow::Result;
use crossterm::event::{
    self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use everruns_core::command::{CommandDescriptor, CommandSource};
use everruns_core::events::{Event as RuntimeEvent, EventData, ToolCompletedData};
use everruns_core::message::{ContentPart, MessageRole};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug)]
pub enum Author {
    User,
    Assistant,
    Tool,
    Process,
    Diff,
    System,
}

impl Author {
    fn label(&self) -> &'static str {
        match self {
            Author::User => "you",
            Author::Assistant => "agent",
            Author::Tool => "tool",
            Author::Process => "proc",
            Author::Diff => "diff",
            Author::System => "system",
        }
    }
    fn color(&self) -> Color {
        match self {
            Author::User => Color::LightCyan,
            Author::Assistant => Color::LightGreen,
            Author::Tool => Color::Yellow,
            Author::Process => Color::LightBlue,
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

#[derive(Clone, Copy)]
struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandSuggestion {
    completion: String,
    label: String,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        usage: "/help",
        description: "show commands",
    },
    CommandSpec {
        name: "tools",
        usage: "/tools",
        description: "list available tools",
    },
    CommandSpec {
        name: "cwd",
        usage: "/cwd",
        description: "show workspace root",
    },
    CommandSpec {
        name: "model",
        usage: "/model <provider>/<id>",
        description: "show or change the active provider/model",
    },
    CommandSpec {
        name: "clear",
        usage: "/clear",
        description: "clear transcript",
    },
    CommandSpec {
        name: "quit",
        usage: "/quit",
        description: "exit",
    },
];

pub struct App {
    bundle: Arc<RuntimeBundle>,
    pub lines: Vec<ChatLine>,
    pub input: String,
    pub busy: bool,
    pub should_quit: bool,
    /// Lines scrolled back from the bottom of the transcript. 0 = stuck to bottom.
    pub scroll: u32,
    /// Last rendered total line count for the chat area, used to clamp scroll.
    pub last_total_lines: u32,
    /// Last rendered visible-line height for the chat area, used for page jumps.
    pub last_chat_height: u32,
    busy_frame: u64,
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
            last_total_lines: 0,
            last_chat_height: 0,
            busy_frame: 0,
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
        self.push_system(format!("model: {}", self.bundle.provider_label()));
        self.push_system(format!("tools: {}", self.bundle.tool_names.join(", ")));
        self.push_system(format!(
            "session: {} (log: {}; {} prior event(s) replayed)",
            self.bundle.session_id,
            self.bundle.session_log_path.display(),
            self.bundle.replayed_events,
        ));
        if !self.bundle.capability_commands.is_empty() {
            let names: Vec<String> = self
                .bundle
                .capability_commands
                .iter()
                .map(|c| format!("/{}", c.name))
                .collect();
            self.push_system(format!("capability commands: {}", names.join(", ")));
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

    pub async fn run<B>(&mut self, terminal: &mut Terminal<B>) -> Result<()>
    where
        B: Backend,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        loop {
            if self.busy {
                self.busy_frame = self.busy_frame.wrapping_add(1);
            }
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
                        self.busy_frame = 0;
                        self.rx = None;
                        continue;
                    }
                    Ok(TurnEvent::Failed(err)) => {
                        self.busy = false;
                        self.busy_frame = 0;
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

            // 3) keystrokes / mouse
            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    CrosstermEvent::Key(key) => {
                        if key.kind == KeyEventKind::Release {
                            continue;
                        }
                        self.handle_key(key).await;
                    }
                    CrosstermEvent::Mouse(m) => self.handle_mouse(m),
                    _ => {}
                }
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

        // Keys that always work, whether or not a turn is running:
        // exit (Esc) and transcript scroll. Input editing is gated below.
        match key.code {
            KeyCode::Esc => {
                self.should_quit = true;
                return;
            }
            KeyCode::Up => {
                self.scroll_by(1);
                return;
            }
            KeyCode::Down => {
                self.scroll_by(-1);
                return;
            }
            KeyCode::PageUp => {
                self.scroll_by(self.page_step() as i32);
                return;
            }
            KeyCode::PageDown => {
                self.scroll_by(-(self.page_step() as i32));
                return;
            }
            KeyCode::Home => {
                self.scroll_to_top();
                return;
            }
            KeyCode::End => {
                self.scroll = 0;
                return;
            }
            _ => {}
        }

        if self.busy {
            // Block only input editing while a turn is running.
            return;
        }
        match key.code {
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Tab => {
                if let Some(suggestion) = self.suggestions().first() {
                    self.input = suggestion.completion.clone();
                }
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.input);
                let text = text.trim().to_string();
                if text.is_empty() {
                    return;
                }
                if let Some(rest) = text.strip_prefix('/') {
                    self.handle_command(rest).await;
                    return;
                }
                self.push_user(text.clone());
                self.start_turn(text);
            }
            _ => {}
        }
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_by(3),
            MouseEventKind::ScrollDown => self.scroll_by(-3),
            _ => {}
        }
    }

    fn page_step(&self) -> u32 {
        (self.last_chat_height / 2).max(1)
    }

    fn scroll_by(&mut self, delta: i32) {
        let max = self.max_scroll();
        let cur = self.scroll as i64 + delta as i64;
        self.scroll = cur.clamp(0, max as i64) as u32;
    }

    fn scroll_to_top(&mut self) {
        self.scroll = self.max_scroll();
    }

    fn max_scroll(&self) -> u32 {
        self.last_total_lines.saturating_sub(self.last_chat_height)
    }

    fn suggestions(&self) -> Vec<CommandSuggestion> {
        command_suggestions(
            &self.input,
            self.bundle.model_suggestions(),
            &self.bundle.capability_commands,
        )
    }

    async fn handle_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        let mut parts = cmd.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap_or_default();
        let arg = parts.next().unwrap_or_default().trim();
        match head {
            "help" => {
                self.push_system(
                    COMMANDS
                        .iter()
                        .map(|cmd| cmd.usage)
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
                if !self.bundle.capability_commands.is_empty() {
                    let caps = self
                        .bundle
                        .capability_commands
                        .iter()
                        .map(capability_command_usage)
                        .collect::<Vec<_>>()
                        .join(" · ");
                    self.push_system(format!("capability commands: {caps}"));
                }
                self.push_system(
                    "scroll: ↑/↓ line · PgUp/PgDn half-page · Home/End top/bottom · mouse wheel"
                        .into(),
                );
                self.push_system("approvals: y allow · n / Esc deny · exit: Esc / Ctrl-D".into());
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
                if arg.is_empty() {
                    self.push_system(format!("model: {}", self.bundle.provider_label()));
                    self.push_system(format!(
                        "usage: /model <provider>/<id>; suggestions: {}",
                        self.bundle.model_suggestions().join(", ")
                    ));
                } else {
                    match self.bundle.set_model(arg).await {
                        Ok(label) => self.push_system(format!("model changed: {label}")),
                        Err(err) => self.push_system(format!("model change failed: {err}")),
                    }
                }
            }
            "clear" => {
                self.lines.clear();
                self.emit_system_banner();
            }
            "quit" | "exit" => self.should_quit = true,
            other => {
                if let Some(descriptor) = self
                    .bundle
                    .capability_commands
                    .iter()
                    .find(|c| c.name == other)
                {
                    self.invoke_capability_command(descriptor.clone(), arg.to_string());
                } else {
                    self.push_system(format!("unknown command: /{other}"));
                }
            }
        }
    }

    /// Send a capability-provided slash command as a regular chat message.
    ///
    /// In the server, system commands route through a dedicated execute endpoint
    /// that bypasses chat history (see `crates/server/src/api/commands.rs`). The
    /// CLI example doesn't have that out-of-band path, so we surface the command
    /// inline: the model sees `/name args` as a user prompt and reacts. Skill
    /// commands behave the same way in the UI today, so this matches the skill
    /// flow exactly and demonstrates the discoverability contract.
    fn invoke_capability_command(&mut self, descriptor: CommandDescriptor, args: String) {
        let trimmed = args.trim();
        let text = if trimmed.is_empty() {
            format!("/{}", descriptor.name)
        } else {
            format!("/{} {trimmed}", descriptor.name)
        };
        let required_missing = descriptor
            .args
            .iter()
            .any(|a| a.required && trimmed.is_empty());
        if required_missing {
            let needed: Vec<&str> = descriptor
                .args
                .iter()
                .filter(|a| a.required)
                .map(|a| a.name.as_str())
                .collect();
            self.push_system(format!(
                "/{} requires: {}",
                descriptor.name,
                needed.join(", ")
            ));
            return;
        }
        let source_label = match descriptor.source {
            CommandSource::System => "system",
            CommandSource::Skill => "skill",
        };
        self.push_system(format!(
            "invoking capability command /{} ({source_label})",
            descriptor.name
        ));
        self.push_user(text.clone());
        self.start_turn(text);
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

            let runtime = bundle.runtime.clone();
            let turn = tokio::spawn(async move { runtime.run_text_turn(session_id, prompt).await });
            let mut emitted_events = HashSet::new();
            while !turn.is_finished() {
                emit_new_process_lines(&bundle, events_before, &mut emitted_events, &tx).await;
                tokio::time::sleep(Duration::from_millis(120)).await;
            }

            let result = match turn.await {
                Ok(result) => result,
                Err(e) => {
                    let _ = tx.send(TurnEvent::Failed(format!("turn task: {e}")));
                    let _ = tx.send(TurnEvent::Done);
                    return;
                }
            };
            emit_new_process_lines(&bundle, events_before, &mut emitted_events, &tx).await;
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
            // Catch any final tool events missed by the polling loop.
            for event in events.iter().skip(events_before) {
                let event_id = event.id.to_string();
                if emitted_events.insert(event_id) {
                    out.extend(process_lines_for_event(event));
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

async fn emit_new_process_lines(
    bundle: &RuntimeBundle,
    events_before: usize,
    emitted_events: &mut HashSet<String>,
    tx: &mpsc::UnboundedSender<TurnEvent>,
) {
    let events = bundle.runtime.events().await.unwrap_or_default();
    let mut lines = Vec::new();
    for event in events.iter().skip(events_before) {
        let event_id = event.id.to_string();
        if emitted_events.insert(event_id) {
            lines.extend(process_lines_for_event(event));
        }
    }
    if !lines.is_empty() {
        let _ = tx.send(TurnEvent::Lines(lines));
    }
}

fn process_lines_for_event(event: &RuntimeEvent) -> Vec<ChatLine> {
    match &event.data {
        EventData::ReasonStarted(_) => vec![process_line("thinking")],
        EventData::ReasonCompleted(data) => {
            if !data.success {
                let err = data.error.as_deref().unwrap_or("reasoning failed");
                return vec![process_line(format!(
                    "reasoning failed: {}",
                    first_line(err, 100)
                ))];
            }
            if data.has_tool_calls {
                vec![process_line(format!(
                    "planned {} tool call(s)",
                    data.tool_call_count
                ))]
            } else {
                vec![process_line("response ready")]
            }
        }
        EventData::ActStarted(data) => {
            let text = data
                .headline
                .clone()
                .unwrap_or_else(|| format!("running {} tool(s)", data.tool_calls.len()));
            vec![process_line(text)]
        }
        EventData::ActCompleted(data) => {
            let text = data.headline.clone().unwrap_or_else(|| {
                format!(
                    "tools finished: {} ok, {} failed",
                    data.success_count, data.error_count
                )
            });
            vec![process_line(text)]
        }
        EventData::ToolStarted(data) => vec![process_line(format!(
            "→ {}",
            data.narration
                .as_deref()
                .or(data.display_name.as_deref())
                .unwrap_or(data.tool_call.name.as_str())
        ))],
        EventData::ToolProgress(data) => vec![process_line(format!(
            "… {}: {}",
            data.display_name
                .as_deref()
                .unwrap_or(data.tool_name.as_str()),
            first_line(&data.message, 100)
        ))],
        EventData::ToolCompleted(data) => {
            let marker = if data.success { "✓" } else { "✗" };
            let label = data
                .narration
                .as_deref()
                .or(data.display_name.as_deref())
                .unwrap_or(data.tool_name.as_str());
            let summary = summarize_tool_result(data);
            let mut lines = vec![ChatLine {
                author: Author::Tool,
                text: if summary.is_empty() {
                    format!("{marker} {label}")
                } else {
                    format!("{marker} {label}  {summary}")
                },
            }];
            if data.tool_name == "edit_file"
                && let Some(diff) = extract_field(data, "diff")
            {
                for line in diff.lines().take(40) {
                    lines.push(ChatLine {
                        author: Author::Diff,
                        text: line.to_string(),
                    });
                }
            }
            lines
        }
        EventData::ToolCallRequested(data) => vec![process_line(format!(
            "waiting for {} client tool result(s)",
            data.tool_calls.len()
        ))],
        EventData::OutputMessageStarted(data) => {
            let suffix = data
                .iteration
                .filter(|iteration| *iteration > 1)
                .map(|iteration| format!(" (iteration {iteration})"))
                .unwrap_or_default();
            vec![process_line(format!("writing response{suffix}"))]
        }
        EventData::ReasonThinkingStarted(_) => vec![process_line("thinking deeply")],
        EventData::TurnCancelled(_) => vec![process_line("turn cancelled")],
        EventData::TurnFailed(data) => vec![process_line(format!(
            "turn failed: {}",
            first_line(&data.error, 100)
        ))],
        _ => Vec::new(),
    }
}

fn process_line(text: impl Into<String>) -> ChatLine {
    ChatLine {
        author: Author::Process,
        text: text.into(),
    }
}

fn command_suggestions(
    input: &str,
    model_suggestions: &[&str],
    capability_commands: &[CommandDescriptor],
) -> Vec<CommandSuggestion> {
    let Some(rest) = input.strip_prefix('/') else {
        return Vec::new();
    };

    if let Some(model_prefix) = rest.strip_prefix("model ") {
        return model_suggestions
            .iter()
            .filter(|model| model.starts_with(model_prefix.trim_start()))
            .take(5)
            .map(|model| CommandSuggestion {
                completion: format!("/model {model}"),
                label: format!("/model {model}    change active provider/model"),
            })
            .collect();
    }

    if rest == "model" {
        return vec![CommandSuggestion {
            completion: "/model ".to_string(),
            label: "/model <provider>/<id>    show or change the active provider/model".to_string(),
        }];
    }

    let mut out: Vec<CommandSuggestion> = COMMANDS
        .iter()
        .filter(|cmd| cmd.name.starts_with(rest))
        .map(|cmd| CommandSuggestion {
            completion: cmd
                .usage
                .split_whitespace()
                .next()
                .unwrap_or(cmd.usage)
                .to_string(),
            label: format!("{}    {}", cmd.usage, cmd.description),
        })
        .collect();

    // Capability-provided commands. Names that collide with a built-in CLI
    // command are skipped (built-in wins) so the local handler keeps running.
    let builtin_names: std::collections::HashSet<&str> = COMMANDS.iter().map(|c| c.name).collect();
    for descriptor in capability_commands {
        if !descriptor.name.starts_with(rest) {
            continue;
        }
        if builtin_names.contains(descriptor.name.as_str()) {
            continue;
        }
        let usage = capability_command_usage(descriptor);
        // If the command takes args, leave a trailing space so the user can
        // start typing immediately after accepting the suggestion.
        let completion = if descriptor.args.is_empty() {
            format!("/{}", descriptor.name)
        } else {
            format!("/{} ", descriptor.name)
        };
        out.push(CommandSuggestion {
            completion,
            label: format!("{usage}    {}", descriptor.description),
        });
    }

    out.truncate(5);
    out
}

fn capability_command_usage(descriptor: &CommandDescriptor) -> String {
    if descriptor.args.is_empty() {
        format!("/{}", descriptor.name)
    } else {
        let args = descriptor
            .args
            .iter()
            .map(|a| {
                if a.required {
                    format!("<{}>", a.name)
                } else {
                    format!("[{}]", a.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("/{} {args}", descriptor.name)
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
    // Field names match the built-in `session_file_system` capability's
    // result shapes. See crates/core/src/capabilities/file_system.rs.
    match data.tool_name.as_str() {
        "read_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let total = v.get("total_lines").and_then(Value::as_u64).unwrap_or(0);
            let shown = v.get("lines_shown");
            let start = shown
                .and_then(|s| s.get("start"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let end = shown
                .and_then(|s| s.get("end"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let count = end.saturating_sub(start.saturating_sub(1));
            format!("{path} ({count}/{total} lines)")
        }
        "write_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let bytes = v.get("size_bytes").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({bytes} bytes)")
        }
        "edit_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let n = v.get("applied_edits").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({n} edit(s))")
        }
        "list_directory" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let n = v.get("count").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({n} entries)")
        }
        "grep_files" => {
            let pattern = v.get("pattern").and_then(Value::as_str).unwrap_or("");
            let n = v.get("match_count").and_then(Value::as_u64).unwrap_or(0);
            format!("/{pattern}/ ({n} match(es))")
        }
        "delete_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            format!("{path} (deleted)")
        }
        "stat_file" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or("");
            let size = v.get("size_bytes").and_then(Value::as_u64).unwrap_or(0);
            format!("{path} ({size} bytes)")
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

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let has_approval = app.pending.is_some();
    let suggestions = if !app.busy && !has_approval {
        app.suggestions()
    } else {
        Vec::new()
    };
    let has_suggestions = !suggestions.is_empty();
    let input_height: u16 = 3;
    let approval_height: u16 = if has_approval { 3 } else { 0 };
    let suggestions_height: u16 = if has_suggestions { 3 } else { 0 };

    let mut constraints = vec![Constraint::Min(3)];
    if has_approval {
        constraints.push(Constraint::Length(approval_height));
    }
    if has_suggestions {
        constraints.push(Constraint::Length(suggestions_height));
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
        draw_approval(f, chunks[idx], &*app);
        idx += 1;
    }
    if has_suggestions {
        draw_suggestions(f, chunks[idx], &suggestions);
        idx += 1;
    }
    draw_input(f, chunks[idx], &*app);
    idx += 1;
    draw_status(f, chunks[idx], &*app);
}

fn draw_chat(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
    app.last_total_lines = total as u32;
    app.last_chat_height = height as u32;
    let max_scroll = total.saturating_sub(height);
    if (app.scroll as usize) > max_scroll {
        app.scroll = max_scroll as u32;
    }
    let scroll_back = app.scroll as usize;
    let end = total.saturating_sub(scroll_back);
    let start = end.saturating_sub(height);
    let view: Vec<Line> = lines[start..end].to_vec();

    let title = if scroll_back == 0 {
        " Everruns Coding CLI ".to_string()
    } else {
        format!(" Everruns Coding CLI · ↑{scroll_back}/{max_scroll}  ↑↓ PgUp/PgDn Home/End ")
    };
    let block = Block::default().borders(Borders::ALL).title(title);
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

fn draw_suggestions(f: &mut ratatui::Frame, area: Rect, suggestions: &[CommandSuggestion]) {
    let text = suggestions
        .iter()
        .map(|suggestion| suggestion.label.as_str())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let para = Paragraph::new(text)
        .style(Style::default().fg(Color::LightBlue))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" suggestions (Tab to accept) "),
        );
    f.render_widget(para, area);
}

fn draw_input(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let title = input_title(app);
    let para = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
    if !app.busy && app.pending.is_none() {
        let x = area.x + 1 + (app.input.len() as u16).min(area.width.saturating_sub(2));
        let y = area.y + 1;
        f.set_cursor_position((x, y));
    }
}

fn input_title(app: &App) -> Line<'static> {
    if app.pending.is_some() {
        return Line::from(" approval pending — answer y / n above ");
    }
    if app.busy {
        return thinking_title(app.busy_frame);
    }
    Line::from(" message (Enter to send) ")
}

fn thinking_title(frame: u64) -> Line<'static> {
    const TEXT: &str = "thinking...";
    const COLORS: [Color; 5] = [
        Color::LightCyan,
        Color::Cyan,
        Color::LightBlue,
        Color::Magenta,
        Color::LightMagenta,
    ];

    let offset = ((frame / 2) as usize) % COLORS.len();
    let mut spans = Vec::with_capacity(TEXT.len() + 2);
    spans.push(Span::raw(" "));
    for (i, ch) in TEXT.chars().enumerate() {
        let color = COLORS[(i + offset) % COLORS.len()];
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        " (input disabled) ",
        Style::default().fg(Color::DarkGray),
    ));
    Line::from(spans)
}

fn draw_status(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let status = format!(
        " {} · {} · {}msgs ",
        app.bundle.provider_label(),
        app.bundle.workspace_root.display(),
        app.lines.len()
    );
    let para = Paragraph::new(Span::styled(status, Style::default().fg(Color::DarkGray)));
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    use everruns_core::command::{CommandArg, CommandDescriptor, CommandSource};

    #[test]
    fn command_suggestions_list_commands_for_slash() {
        let suggestions = command_suggestions("/", &["openai/gpt-5.5"], &[]);

        assert!(suggestions.iter().any(|s| s.completion == "/help"));
        assert!(suggestions.iter().any(|s| s.completion == "/model"));
    }

    #[test]
    fn command_suggestions_complete_model_command_before_args() {
        let suggestions = command_suggestions("/model", &["openai/gpt-5.5"], &[]);

        assert_eq!(
            suggestions,
            vec![CommandSuggestion {
                completion: "/model ".to_string(),
                label: "/model <provider>/<id>    show or change the active provider/model"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn command_suggestions_filter_models_by_prefix() {
        let suggestions = command_suggestions(
            "/model openai/gpt-5.",
            &[
                "openai/gpt-5.5",
                "openai/gpt-5.4-mini",
                "anthropic/claude-sonnet-4-5",
            ],
            &[],
        );

        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.completion.as_str())
                .collect::<Vec<_>>(),
            vec!["/model openai/gpt-5.5", "/model openai/gpt-5.4-mini"]
        );
    }

    #[test]
    fn capability_commands_appear_in_suggestions() {
        let caps = vec![CommandDescriptor {
            name: "btw".to_string(),
            description: "Ask a side question.".to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "question".to_string(),
                description: "the question".to_string(),
                required: true,
            }],
        }];

        let suggestions = command_suggestions("/b", &[], &caps);

        let btw = suggestions
            .iter()
            .find(|s| s.completion == "/btw ")
            .expect("capability command surfaced in suggestions");
        assert!(btw.label.starts_with("/btw <question>"));
    }

    #[test]
    fn builtin_commands_win_over_capability_with_same_name() {
        // A capability that accidentally declares /help must not shadow the
        // built-in handler: the built-in suggestion (no trailing space, no
        // args) should be the only one returned for that name.
        let caps = vec![CommandDescriptor {
            name: "help".to_string(),
            description: "shadow help".to_string(),
            source: CommandSource::System,
            args: vec![],
        }];

        let suggestions = command_suggestions("/help", &[], &caps);

        let help_entries: Vec<_> = suggestions
            .iter()
            .filter(|s| s.completion.starts_with("/help"))
            .collect();
        assert_eq!(help_entries.len(), 1);
        assert_eq!(help_entries[0].completion, "/help");
    }
}
