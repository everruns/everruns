// TUI app state and event loop.
// Decision: keep the rendering layer tiny — a scrolling chat above a single
// input box, plus a modal approval bar for destructive tools. The event loop
// multiplexes terminal keystrokes with a background turn task and an approval
// channel so the UI stays responsive while the model is thinking.

use crate::approval::ApprovalRequest;
use crate::runtime::{BuiltRuntime, ModelState, RuntimeHandles, StartupInfo};
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
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug)]
pub enum Author {
    User,
    Assistant,
    Tool,
    ToolDetail,
    Diff,
    System,
}

impl Author {
    pub fn label(&self) -> &'static str {
        match self {
            Author::User => "you",
            Author::Assistant => "agent",
            Author::Tool => "tool",
            Author::ToolDetail => "",
            Author::Diff => "diff",
            Author::System => "system",
        }
    }
    pub fn color(&self) -> Color {
        match self {
            Author::User => Color::LightCyan,
            Author::Assistant => Color::LightGreen,
            Author::Tool => Color::Yellow,
            Author::ToolDetail => Color::Gray,
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
    handles: RuntimeHandles,
    startup: StartupInfo,
    model: ModelState,
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
    turn_activity: Option<String>,
    rx: Option<mpsc::UnboundedReceiver<TurnEvent>>,
    approval_rx: ApprovalRx,
    pending: Option<PendingApproval>,
}

#[derive(Clone, Debug)]
pub struct ActivityStatus {
    pub text: String,
    fallback: bool,
}

#[derive(Debug)]
enum TurnEvent {
    Lines(Vec<ChatLine>),
    Activity(ActivityStatus),
    Done,
    Failed(String),
}

impl App {
    pub fn new(runtime: BuiltRuntime, approval_rx: ApprovalRx) -> Self {
        let mut app = Self {
            handles: runtime.handles,
            startup: runtime.startup,
            model: runtime.model,
            lines: Vec::new(),
            input: String::new(),
            busy: false,
            should_quit: false,
            scroll: 0,
            last_total_lines: 0,
            last_chat_height: 0,
            busy_frame: 0,
            turn_activity: None,
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
            self.startup.workspace_root.display()
        ));
        self.push_system(format!("model: {}", self.model.provider_label()));
        self.push_system(format!("tools: {}", self.startup.tool_names.join(", ")));
        self.push_system(format!(
            "session: {} (log: {}; {} prior event(s) replayed)",
            self.handles.session_id,
            self.startup.session_log_path.display(),
            self.startup.replayed_events,
        ));
        if !self.startup.capability_commands.is_empty() {
            let names: Vec<String> = self
                .startup
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
                    Ok(TurnEvent::Activity(activity)) => {
                        if !activity.fallback || self.turn_activity.is_none() {
                            self.turn_activity = Some(activity.text);
                        }
                        continue;
                    }
                    Ok(TurnEvent::Done) => {
                        self.busy = false;
                        self.busy_frame = 0;
                        self.turn_activity = None;
                        self.rx = None;
                        continue;
                    }
                    Ok(TurnEvent::Failed(err)) => {
                        self.busy = false;
                        self.busy_frame = 0;
                        self.turn_activity = None;
                        self.rx = None;
                        self.push_system(format!("turn failed: {err}"));
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        self.busy = false;
                        self.turn_activity = None;
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
        command_suggestions(&self.input, &self.startup.capability_commands)
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
                if !self.startup.capability_commands.is_empty() {
                    let caps = self
                        .startup
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
                self.push_system(format!("tools: {}", self.startup.tool_names.join(", ")));
            }
            "cwd" => {
                self.push_system(format!(
                    "workspace root: {}",
                    self.startup.workspace_root.display()
                ));
            }
            "clear" => {
                self.lines.clear();
                self.emit_system_banner();
            }
            "quit" | "exit" => self.should_quit = true,
            other => {
                if let Some(descriptor) = self
                    .startup
                    .capability_commands
                    .iter()
                    .find(|c| c.name == other)
                    .cloned()
                {
                    self.invoke_capability_command(descriptor, arg.to_string())
                        .await;
                } else {
                    self.push_system(format!("unknown command: /{other}"));
                }
            }
        }
    }

    /// Dispatch a capability-provided slash command.
    ///
    /// `System` commands execute through `runtime.execute_command` — the
    /// capability's own handler runs and the result is rendered inline. This
    /// is the path `/model` now takes. `Skill` commands match the web UI's
    /// behavior: the literal `/name args` text is sent as a chat message so
    /// the LLM activates the skill.
    async fn invoke_capability_command(&mut self, descriptor: CommandDescriptor, args: String) {
        let trimmed = args.trim();
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

        match descriptor.source {
            CommandSource::System => {
                let request = everruns_core::command::ExecuteCommandRequest {
                    name: descriptor.name.clone(),
                    arguments: if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    },
                    controls: None,
                };
                let result = self
                    .handles
                    .runtime
                    .execute_command(self.handles.session_id, request)
                    .await;
                match result {
                    Ok(result) => {
                        let prefix = if result.success { "" } else { "error: " };
                        self.push_system(format!("{prefix}{}", result.message));
                    }
                    Err(err) => self.push_system(format!("/{} failed: {err}", descriptor.name)),
                }
            }
            CommandSource::Skill => {
                let text = if trimmed.is_empty() {
                    format!("/{}", descriptor.name)
                } else {
                    format!("/{} {trimmed}", descriptor.name)
                };
                self.push_user(text.clone());
                self.start_turn(text);
            }
        }
    }

    fn start_turn(&mut self, prompt: String) {
        let handles = self.handles.clone();
        let model = self.model.clone();
        let (tx, rx) = mpsc::unbounded_channel::<TurnEvent>();
        self.rx = Some(rx);
        self.busy = true;
        self.turn_activity = None;

        tokio::spawn(async move {
            let session_id = handles.session_id;
            let before = match handles.runtime.messages(session_id).await {
                Ok(m) => m.len(),
                Err(e) => {
                    let _ = tx.send(TurnEvent::Failed(format!("load history: {e}")));
                    let _ = tx.send(TurnEvent::Done);
                    return;
                }
            };
            let events_before = match handles.runtime.events().await {
                Ok(e) => e.len(),
                Err(_) => 0,
            };

            let input = model.input_message(prompt);
            let runtime = handles.runtime.clone();
            let turn = tokio::spawn(async move { runtime.run_turn(session_id, input).await });
            let mut emitted_events = HashSet::new();
            while !turn.is_finished() {
                emit_new_turn_events(&handles, events_before, &mut emitted_events, &tx).await;
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
            emit_new_turn_events(&handles, events_before, &mut emitted_events, &tx).await;
            let response = match result {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(TurnEvent::Failed(format!("{e}")));
                    let _ = tx.send(TurnEvent::Done);
                    return;
                }
            };

            let messages = handles
                .runtime
                .messages(session_id)
                .await
                .unwrap_or_default();
            let events = handles.runtime.events().await.unwrap_or_default();

            let mut out = Vec::new();
            // Catch any final tool events missed by the polling loop.
            for event in events.iter().skip(events_before) {
                let event_id = event.id.to_string();
                if emitted_events.insert(event_id) {
                    if let Some(activity) = status_for_event(event) {
                        let _ = tx.send(TurnEvent::Activity(activity));
                    }
                    out.extend(lines_for_event(event));
                }
            }
            // Assistant text from the turn.
            for msg in messages.iter().skip(before) {
                if msg.role == MessageRole::Agent
                    && !msg.has_tool_calls()
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

async fn emit_new_turn_events(
    handles: &RuntimeHandles,
    events_before: usize,
    emitted_events: &mut HashSet<String>,
    tx: &mpsc::UnboundedSender<TurnEvent>,
) {
    let events = handles.runtime.events().await.unwrap_or_default();
    let mut lines = Vec::new();
    for event in events.iter().skip(events_before) {
        let event_id = event.id.to_string();
        if emitted_events.insert(event_id) {
            if let Some(activity) = status_for_event(event) {
                let _ = tx.send(TurnEvent::Activity(activity));
            }
            lines.extend(lines_for_event(event));
        }
    }
    if !lines.is_empty() {
        let _ = tx.send(TurnEvent::Lines(lines));
    }
}

pub fn lines_for_event(event: &RuntimeEvent) -> Vec<ChatLine> {
    match &event.data {
        EventData::ReasonStarted(_) => Vec::new(),
        EventData::ReasonCompleted(data) => {
            if data.success && data.has_tool_calls {
                let mut lines = Vec::new();
                if let Some(text) = data
                    .text_preview
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    lines.push(ChatLine {
                        author: Author::Assistant,
                        text: text.to_string(),
                    });
                }
                lines
            } else {
                Vec::new()
            }
        }
        EventData::OutputMessageCompleted(data) => thinking_lines_for_message(&data.message),
        EventData::ToolCompleted(data) => {
            if data.tool_name == "write_todos" {
                return todo_lines_for_result(data);
            }
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
        _ => Vec::new(),
    }
}

fn thinking_lines_for_message(message: &everruns_core::Message) -> Vec<ChatLine> {
    if message.role != MessageRole::Agent || !message.has_tool_calls() {
        return Vec::new();
    }
    message
        .thinking
        .as_deref()
        .map(str::trim)
        .filter(|thinking| !thinking.is_empty())
        .map(|thinking| {
            vec![ChatLine {
                author: Author::Assistant,
                text: thinking.to_string(),
            }]
        })
        .unwrap_or_default()
}

pub fn status_for_event(event: &RuntimeEvent) -> Option<ActivityStatus> {
    match &event.data {
        EventData::ReasonStarted(_) => Some(fallback_status("thinking")),
        EventData::ReasonCompleted(data) => {
            if !data.success {
                let err = data.error.as_deref().unwrap_or("reasoning failed");
                return Some(activity_status(format!(
                    "reasoning failed: {}",
                    first_line(err, 100)
                )));
            }
            data.has_tool_calls
                .then(|| activity_status(format!("planned {} tool call(s)", data.tool_call_count)))
        }
        EventData::ActStarted(data) => data
            .headline
            .clone()
            .or_else(|| Some(format!("running {} tool(s)", data.tool_calls.len())))
            .map(activity_status),
        EventData::ActCompleted(data) => data
            .headline
            .clone()
            .or_else(|| {
                Some(format!(
                    "tools finished: {} ok, {} failed",
                    data.success_count, data.error_count
                ))
            })
            .map(activity_status),
        EventData::ToolStarted(data) => Some(activity_status(format!(
            "→ {}",
            data.narration
                .as_deref()
                .or(data.display_name.as_deref())
                .unwrap_or(data.tool_call.name.as_str())
        ))),
        EventData::ToolProgress(data) => Some(activity_status(format!(
            "… {}: {}",
            data.display_name
                .as_deref()
                .unwrap_or(data.tool_name.as_str()),
            first_line(&data.message, 100)
        ))),
        EventData::ToolCallRequested(data) => Some(activity_status(format!(
            "waiting for {} client tool result(s)",
            data.tool_calls.len()
        ))),
        EventData::OutputMessageStarted(data) => {
            let iteration = data.iteration.unwrap_or(1);
            Some(activity_status(format!(
                "iteration {iteration}: writing response"
            )))
        }
        EventData::ReasonThinkingStarted(_) => Some(fallback_status("thinking deeply")),
        EventData::TurnCancelled(_) => Some(activity_status("turn cancelled")),
        EventData::TurnFailed(data) => Some(activity_status(format!(
            "turn failed: {}",
            first_line(&data.error, 100)
        ))),
        _ => None,
    }
}

fn activity_status(text: impl Into<String>) -> ActivityStatus {
    ActivityStatus {
        text: text.into(),
        fallback: false,
    }
}

fn fallback_status(text: impl Into<String>) -> ActivityStatus {
    ActivityStatus {
        text: text.into(),
        fallback: true,
    }
}

fn command_suggestions(
    input: &str,
    capability_commands: &[CommandDescriptor],
) -> Vec<CommandSuggestion> {
    let Some(rest) = input.strip_prefix('/') else {
        return Vec::new();
    };

    // If the user already typed a command name and a space, surface the
    // first-arg suggestions declared by the matching capability. This is
    // fully declarative — the capability populates `CommandArg::suggestions`
    // when it builds its `CommandDescriptor`, so the UI never has to call
    // back into the capability between keystrokes.
    if let Some((head, arg_prefix)) = rest.split_once(' ')
        && let Some(descriptor) = capability_commands.iter().find(|c| c.name == head)
        && let Some(arg) = descriptor.args.first()
        && !arg.suggestions.is_empty()
    {
        let prefix = arg_prefix.trim_start();
        return arg
            .suggestions
            .iter()
            .filter(|s| s.starts_with(prefix))
            .take(8)
            .map(|s| CommandSuggestion {
                completion: format!("/{} {s}", descriptor.name),
                label: format!("/{} {s}    {}", descriptor.name, descriptor.description),
            })
            .collect();
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

    out.truncate(8);
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

fn todo_lines_for_result(data: &ToolCompletedData) -> Vec<ChatLine> {
    let Some(v) = result_value(data) else {
        return vec![ChatLine {
            author: Author::Tool,
            text: format!(
                "✓ {}",
                data.display_name.as_deref().unwrap_or("Write Todos")
            ),
        }];
    };
    let Some(todos) = v.get("todos").and_then(Value::as_array) else {
        return vec![ChatLine {
            author: Author::Tool,
            text: summarize_tool_result(data),
        }];
    };

    let total = todos.len();
    let completed = todos
        .iter()
        .filter(|todo| todo.get("status").and_then(Value::as_str) == Some("completed"))
        .count();
    let mut lines = vec![ChatLine {
        author: Author::Tool,
        text: format!("{completed} of {total} todos completed"),
    }];

    for todo in todos {
        let status = todo
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        let content = todo.get("content").and_then(Value::as_str).unwrap_or("");
        let active_form = todo
            .get("activeForm")
            .and_then(Value::as_str)
            .unwrap_or(content);
        let (marker, text) = match status {
            "completed" => ("[x]", content),
            "in_progress" => ("[>]", active_form),
            _ => ("[ ]", content),
        };
        lines.push(ChatLine {
            author: Author::ToolDetail,
            text: format!("{marker} {text}"),
        });
    }

    if let Some(warning) = v.get("warning").and_then(Value::as_str) {
        lines.push(ChatLine {
            author: Author::ToolDetail,
            text: format!("warning: {warning}"),
        });
    }

    lines
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
        "write_todos" => {
            let completed = v.get("completed").and_then(Value::as_u64).unwrap_or(0);
            let total = v.get("total_tasks").and_then(Value::as_u64).unwrap_or(0);
            format!("{completed}/{total} completed")
        }
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
        append_chat_lines(&mut lines, chat, inner_width);
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

fn append_chat_lines<'a>(lines: &mut Vec<Line<'a>>, chat: &ChatLine, inner_width: usize) {
    if matches!(chat.author, Author::ToolDetail) {
        append_wrapped_plain(
            lines,
            "           ",
            Style::default().fg(Color::Gray),
            &chat.text,
            inner_width,
        );
        return;
    }

    let header_text = format!("{} › ", chat.author.label());
    let header_style = Style::default()
        .fg(chat.author.color())
        .add_modifier(Modifier::BOLD);
    if matches!(chat.author, Author::Assistant) {
        append_markdown_lines(lines, &header_text, header_style, &chat.text, inner_width);
    } else {
        append_wrapped_plain(lines, &header_text, header_style, &chat.text, inner_width);
    }
}

fn append_wrapped_plain<'a>(
    lines: &mut Vec<Line<'a>>,
    first_prefix: &str,
    prefix_style: Style,
    text: &str,
    inner_width: usize,
) {
    let continuation = " ".repeat(first_prefix.chars().count());
    let wrap_width = inner_width
        .saturating_sub(first_prefix.chars().count())
        .max(20);
    let mut emitted = false;
    for raw in text.lines() {
        let wrapped = textwrap::wrap(raw, wrap_width);
        if wrapped.is_empty() {
            let prefix = if emitted {
                continuation.as_str()
            } else {
                first_prefix
            };
            lines.push(Line::from(vec![Span::styled(
                prefix.to_string(),
                prefix_style,
            )]));
            emitted = true;
            continue;
        }
        for piece in wrapped {
            let prefix = if emitted {
                continuation.as_str()
            } else {
                first_prefix
            };
            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), prefix_style),
                Span::raw(piece.into_owned()),
            ]));
            emitted = true;
        }
    }
    if !emitted {
        lines.push(Line::from(vec![Span::styled(
            first_prefix.to_string(),
            prefix_style,
        )]));
    }
}

fn append_markdown_lines<'a>(
    lines: &mut Vec<Line<'a>>,
    first_prefix: &str,
    prefix_style: Style,
    text: &str,
    inner_width: usize,
) {
    let continuation = " ".repeat(first_prefix.chars().count());
    let wrap_width = inner_width
        .saturating_sub(first_prefix.chars().count())
        .max(20);
    let mut first = true;
    let mut in_code = false;

    for raw in text.lines() {
        let trimmed = raw.trim_end();
        if let Some(lang) = trimmed.trim_start().strip_prefix("```") {
            in_code = !in_code;
            let code_lang = lang.trim();
            let label = if in_code {
                if code_lang.is_empty() {
                    "code".to_string()
                } else {
                    format!("code: {code_lang}")
                }
            } else {
                String::new()
            };
            push_markdown_line(
                lines,
                first_prefix,
                &continuation,
                prefix_style,
                &mut first,
                vec![Span::styled(
                    label,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )],
            );
            continue;
        }

        let content_spans = if in_code {
            markdown_code_spans(trimmed)
        } else {
            markdown_text_spans(trimmed)
        };
        let plain = spans_plain_text(&content_spans);
        let wrapped = textwrap::wrap(&plain, wrap_width);
        if wrapped.is_empty() {
            push_markdown_line(
                lines,
                first_prefix,
                &continuation,
                prefix_style,
                &mut first,
                vec![],
            );
            continue;
        }
        if content_spans.len() == 1 {
            let style = content_spans[0].style;
            for piece in wrapped {
                push_markdown_line(
                    lines,
                    first_prefix,
                    &continuation,
                    prefix_style,
                    &mut first,
                    vec![Span::styled(piece.into_owned(), style)],
                );
            }
        } else {
            push_markdown_line(
                lines,
                first_prefix,
                &continuation,
                prefix_style,
                &mut first,
                content_spans,
            );
        }
    }
}

fn push_markdown_line<'a>(
    lines: &mut Vec<Line<'a>>,
    first_prefix: &str,
    continuation: &str,
    prefix_style: Style,
    first: &mut bool,
    mut spans: Vec<Span<'a>>,
) {
    let prefix = if *first { first_prefix } else { continuation };
    let mut line_spans = vec![Span::styled(prefix.to_string(), prefix_style)];
    line_spans.append(&mut spans);
    lines.push(Line::from(line_spans));
    *first = false;
}

fn markdown_text_spans(text: &str) -> Vec<Span<'static>> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') {
        let heading = trimmed.trim_start_matches('#').trim_start();
        return vec![Span::styled(
            heading.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )];
    }
    if let Some(rest) = trimmed.strip_prefix("> ") {
        return vec![
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(rest.to_string(), Style::default().fg(Color::Gray)),
        ];
    }
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return vec![
            Span::styled("- ", Style::default().fg(Color::Gray)),
            Span::raw(rest.to_string()),
        ];
    }
    if let Some((marker, rest)) = numbered_marker(trimmed) {
        return vec![
            Span::styled(marker, Style::default().fg(Color::Gray)),
            Span::raw(rest.to_string()),
        ];
    }
    inline_code_spans(text)
}

fn markdown_code_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("    ", Style::default().fg(Color::DarkGray))];
    spans.extend(simple_code_highlight(text));
    spans
}

fn inline_code_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    let mut code = false;
    while let Some((before, after_tick)) = rest.split_once('`') {
        if !before.is_empty() {
            spans.push(Span::raw(before.to_string()));
        }
        if let Some((inside, after)) = after_tick.split_once('`') {
            spans.push(Span::styled(
                inside.to_string(),
                Style::default().fg(Color::White).bg(Color::Black),
            ));
            rest = after;
            code = true;
        } else {
            spans.push(Span::raw("`".to_string()));
            rest = after_tick;
            break;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::raw(rest.to_string()));
    }
    if spans.is_empty() || !code {
        vec![Span::raw(text.to_string())]
    } else {
        spans
    }
}

fn simple_code_highlight(text: &str) -> Vec<Span<'static>> {
    let keywords = [
        "async", "await", "const", "enum", "fn", "impl", "let", "match", "pub", "return", "struct",
        "use",
    ];
    let mut spans = Vec::new();
    let mut token = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            token.push(ch);
            continue;
        }
        if !token.is_empty() {
            let style = if keywords.contains(&token.as_str()) {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if token.chars().all(|c| c.is_ascii_digit()) {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(Color::LightCyan)
            };
            spans.push(Span::styled(std::mem::take(&mut token), style));
        }
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(Color::Gray),
        ));
    }
    if !token.is_empty() {
        let style = if keywords.contains(&token.as_str()) {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else if token.chars().all(|c| c.is_ascii_digit()) {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::LightCyan)
        };
        spans.push(Span::styled(token, style));
    }
    spans
}

fn numbered_marker(text: &str) -> Option<(String, &str)> {
    let dot = text.find(". ")?;
    if dot == 0 || !text[..dot].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((text[..dot + 2].to_string(), &text[dot + 2..]))
}

fn spans_plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.content.as_ref()).collect()
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
        return thinking_title(
            app.busy_frame,
            app.turn_activity.as_deref().unwrap_or("thinking"),
        );
    }
    Line::from(" message (Enter to send) ")
}

fn thinking_title(frame: u64, activity: &str) -> Line<'static> {
    const SPINNER: [&str; 4] = ["-", "\\", "|", "/"];
    let spinner = SPINNER[((frame / 2) as usize) % SPINNER.len()];
    let text = format!("{activity}...");
    let text_style = Style::default()
        .fg(Color::Gray)
        .add_modifier(Modifier::BOLD);
    let spans = vec![
        Span::raw(" "),
        Span::styled(spinner.to_string(), text_style),
        Span::raw(" "),
        Span::styled(text, text_style),
        Span::styled(" (input disabled) ", Style::default().fg(Color::DarkGray)),
    ];
    Line::from(spans)
}

fn draw_status(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let status = format!(
        " {} · {} · {}msgs ",
        app.model.provider_label(),
        app.startup.workspace_root.display(),
        app.lines.len()
    );
    let para = Paragraph::new(Span::styled(status, Style::default().fg(Color::DarkGray)));
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{
        EventContext, OutputMessageCompletedData, OutputMessageStartedData, ReasonCompletedData,
    };
    use everruns_core::tool_types::ToolCall;
    use everruns_core::{SessionId, TurnId};

    use everruns_core::command::{CommandArg, CommandDescriptor, CommandSource};

    fn model_capability_command() -> CommandDescriptor {
        // Mirrors what `ModelSwitcherCapability::commands()` returns: the
        // arg carries its own static suggestion list so the renderer can
        // surface autocomplete entries straight from the descriptor.
        CommandDescriptor {
            name: "model".to_string(),
            description: "Show or change the active provider/model.".to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "spec".to_string(),
                description: "<provider>/<id>".to_string(),
                required: false,
                suggestions: vec![
                    "openai/gpt-5.5".to_string(),
                    "openai/gpt-5.4-mini".to_string(),
                    "anthropic/claude-sonnet-4-5".to_string(),
                ],
            }],
        }
    }

    #[test]
    fn command_suggestions_list_commands_for_slash() {
        let caps = vec![model_capability_command()];
        let suggestions = command_suggestions("/", &caps);

        assert!(suggestions.iter().any(|s| s.completion == "/help"));
        assert!(
            suggestions
                .iter()
                .any(|s| s.completion == "/model" || s.completion == "/model "),
            "capability-provided /model should appear in suggestions: {suggestions:?}"
        );
    }

    #[test]
    fn command_suggestions_filter_first_arg_by_prefix() {
        // After `/model <prefix>`, the suggestion source must be the arg's
        // declared `suggestions` — read straight from the descriptor with
        // no extra plumbing.
        let caps = vec![model_capability_command()];
        let suggestions = command_suggestions("/model openai/gpt-5.", &caps);

        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.completion.as_str())
                .collect::<Vec<_>>(),
            vec!["/model openai/gpt-5.5", "/model openai/gpt-5.4-mini"]
        );
    }

    #[test]
    fn command_suggestions_no_arg_suggestions_means_free_form() {
        // A capability command whose first arg has no suggestions returns an
        // empty list once the user types past the command name — the renderer
        // should fall back to plain text entry instead of fabricating items.
        let caps = vec![CommandDescriptor {
            name: "echo".to_string(),
            description: "echo".to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "text".to_string(),
                description: "text".to_string(),
                required: true,
                suggestions: vec![],
            }],
        }];

        let suggestions = command_suggestions("/echo hello", &caps);
        assert!(suggestions.is_empty(), "got: {suggestions:?}");
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
                suggestions: vec![],
            }],
        }];

        let suggestions = command_suggestions("/b", &caps);

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

        let suggestions = command_suggestions("/help", &caps);

        let help_entries: Vec<_> = suggestions
            .iter()
            .filter(|s| s.completion.starts_with("/help"))
            .collect();
        assert_eq!(help_entries.len(), 1);
        assert_eq!(help_entries[0].completion, "/help");
    }

    #[test]
    fn lines_for_event_surfaces_tool_call_monologue() {
        let event = RuntimeEvent::new(
            SessionId::new(),
            EventContext::empty(),
            ReasonCompletedData::success("I'll check the manifests first.", true, 2, None, None),
        );

        let lines = lines_for_event(&event);

        assert_eq!(lines.len(), 1);
        assert!(matches!(lines[0].author, Author::Assistant));
        assert_eq!(lines[0].text, "I'll check the manifests first.");
        assert_eq!(
            status_for_event(&event)
                .map(|status| status.text)
                .as_deref(),
            Some("planned 2 tool call(s)")
        );
    }

    #[test]
    fn lines_for_event_surfaces_output_message_thinking() {
        let mut message = everruns_core::Message::assistant_with_tools(
            "",
            vec![ToolCall {
                id: "call_read".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "/workspace/Cargo.toml" }),
            }],
        );
        message.thinking = Some(
            "**Inspecting package files**\n\nI should read the package manifest first.".to_string(),
        );
        let event = RuntimeEvent::new(
            SessionId::new(),
            EventContext::empty(),
            OutputMessageCompletedData::new(message),
        );

        let lines = lines_for_event(&event);

        assert_eq!(lines.len(), 1);
        assert!(matches!(lines[0].author, Author::Assistant));
        assert_eq!(
            lines[0].text,
            "**Inspecting package files**\n\nI should read the package manifest first."
        );
    }

    #[test]
    fn status_for_event_labels_output_iteration() {
        let event = RuntimeEvent::new(
            SessionId::new(),
            EventContext::empty(),
            OutputMessageStartedData {
                turn_id: TurnId::new(),
                model: None,
                iteration: Some(3),
            },
        );

        assert!(lines_for_event(&event).is_empty());
        assert_eq!(
            status_for_event(&event)
                .map(|status| status.text)
                .as_deref(),
            Some("iteration 3: writing response")
        );
    }

    #[test]
    fn lines_for_event_renders_write_todos_items() {
        let event = RuntimeEvent::new(
            SessionId::new(),
            EventContext::empty(),
            ToolCompletedData::success(
                "call_todos".to_string(),
                "write_todos".to_string(),
                vec![ContentPart::text(
                    serde_json::json!({
                        "success": true,
                        "total_tasks": 3,
                        "pending": 1,
                        "in_progress": 1,
                        "completed": 1,
                        "todos": [
                            {
                                "content": "Read current CLI renderer",
                                "activeForm": "Reading current CLI renderer",
                                "status": "completed"
                            },
                            {
                                "content": "Render todos in transcript",
                                "activeForm": "Rendering todos in transcript",
                                "status": "in_progress"
                            },
                            {
                                "content": "Run focused tests",
                                "activeForm": "Running focused tests",
                                "status": "pending"
                            }
                        ]
                    })
                    .to_string(),
                )],
                None,
            ),
        );

        let lines = lines_for_event(&event)
            .into_iter()
            .map(|line| (line.author, line.text))
            .collect::<Vec<_>>();

        assert!(matches!(lines[0].0, Author::Tool));
        assert_eq!(lines[0].1, "1 of 3 todos completed");
        assert!(
            lines
                .iter()
                .any(|(author, line)| matches!(author, Author::ToolDetail)
                    && line == "[x] Read current CLI renderer")
        );
        assert!(
            lines
                .iter()
                .any(|(author, line)| matches!(author, Author::ToolDetail)
                    && line == "[>] Rendering todos in transcript")
        );
        assert!(
            lines
                .iter()
                .any(|(author, line)| matches!(author, Author::ToolDetail)
                    && line == "[ ] Run focused tests")
        );
    }
}
