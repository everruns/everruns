//! Friction: what each shell call in a run was spent on.
//!
//! Models do not know our command lines from training, so the calls an agent
//! spends walking `--help` and recovering from rejected guesses are the cost
//! the help text controls. This module splits a run's `bash` calls into:
//!
//! * **help**: every invocation of the surface's command only read a help page
//!   (`--help`, a bare group such as `everruns agents`, or a discovery verb
//!   such as `tools search`);
//! * **rejected**: the command tree refused something in the call (usage error,
//!   unknown command, unknown flag, a guessed name the shell could not find);
//! * **real**: the call ran at least one real command and nothing was refused;
//! * **other**: the call never invoked the surface (`jq` on a file, `cat` a doc).
//!
//! Decisions:
//!
//! - **The unit is the tool call**, because that is what the budget and the
//!   model's latency pay for. Invocations inside a call are listed in the
//!   friction report, but only the call is counted.
//! - **Rejected wins.** A call that read help and then had a guess refused is a
//!   rejection: the refusal is what the tuning loop has to remove.
//! - **Text, not exit codes.** Bare groups exit non-zero yet are help, and a
//!   real command can fail for reasons help cannot fix. A rejection is the
//!   tree's own wording in the output, so it is classified from that.
//! - **Shared, not restated.** The tools-in-shell study includes this file by
//!   path, so both studies count calls the same way. It depends only on
//!   `serde_json` and `regex`, not on a Mira version.

use serde_json::{Value, json};

/// The command surface whose friction is being counted.
pub struct Surface {
    /// The shell command (`everruns`, `tools`).
    pub command: &'static str,
    /// Whether a word path (`agents`, `agents triggers`) names a group whose
    /// bare invocation prints a help page rather than running something.
    pub is_group: fn(&str) -> bool,
    /// First words that read rather than run (`search` for `tools`).
    pub discovery_verbs: &'static [&'static str],
    /// An output line starting with one of these is a rejection.
    pub rejection_prefixes: &'static [&'static str],
    /// An output line containing one of these is a rejection.
    pub rejection_markers: &'static [&'static str],
}

/// What one tool call was spent on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallClass {
    Help,
    Rejected,
    Real,
    Other,
}

impl CallClass {
    pub fn name(self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Rejected => "rejected",
            Self::Real => "real",
            Self::Other => "other",
        }
    }
}

/// One invocation of the surface's command inside a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// The invocation as written, quotes included.
    pub text: String,
    /// Words after the command, unquoted, with redirections dropped.
    pub words: Vec<String>,
    pub help: bool,
}

impl Invocation {
    /// The help page this reads, as a command line without the help flag:
    /// `everruns agents create`.
    fn page(&self, command: &str) -> String {
        std::iter::once(command)
            .chain(
                self.words
                    .iter()
                    .map(String::as_str)
                    .filter(|word| !matches!(*word, "--help" | "-h" | "help")),
            )
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug)]
pub struct Call {
    pub script: String,
    pub output: String,
    pub invocations: Vec<Invocation>,
    pub class: CallClass,
    /// The tree's refusal, trimmed, when the call was rejected.
    pub error: Option<String>,
}

/// Every `bash` call in a run, classified.
#[derive(Clone, Debug, Default)]
pub struct Friction {
    pub calls: Vec<Call>,
}

impl Friction {
    pub fn count(&self, class: CallClass) -> usize {
        self.calls.iter().filter(|call| call.class == class).count()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// `(name, value)` pairs for `Transcript::record_metric`.
    pub fn metrics(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("friction.calls", self.calls.len() as f64),
            ("friction.help_reads", self.count(CallClass::Help) as f64),
            ("friction.rejected", self.count(CallClass::Rejected) as f64),
            ("friction.real", self.count(CallClass::Real) as f64),
            ("friction.other", self.count(CallClass::Other) as f64),
        ]
    }

    /// One line for a report: `5 calls: 2 help, 1 rejected, 2 real, 0 other`.
    pub fn summary(&self) -> String {
        format!(
            "{} calls: {} help, {} rejected, {} real, {} other",
            self.calls.len(),
            self.count(CallClass::Help),
            self.count(CallClass::Rejected),
            self.count(CallClass::Real),
            self.count(CallClass::Other),
        )
    }

    /// The tuning loop's input for one run: the commands tried in order with
    /// their outcome, the refusal each rejected one got, and the help pages
    /// read. `command` is the surface's command name.
    pub fn report(&self, command: &str) -> Value {
        let mut commands = Vec::new();
        let mut help_pages = Vec::new();
        for (index, call) in self.calls.iter().enumerate() {
            // A refused call names the invocation it refused in its usage line
            // or error; without one, the last invocation is the likeliest.
            let culprit = (call.class == CallClass::Rejected).then(|| {
                call.invocations
                    .iter()
                    .rposition(|inv| {
                        !inv.help && {
                            let page = inv.page(command);
                            let head: Vec<&str> = page.split_whitespace().take(3).collect();
                            call.output.contains(&head.join(" "))
                        }
                    })
                    .or_else(|| call.invocations.iter().rposition(|inv| !inv.help))
                    .or_else(|| call.invocations.len().checked_sub(1))
            });
            if call.invocations.is_empty() {
                commands.push(json!({
                    "call": index,
                    "command": first_line(&call.script),
                    "kind": "unknown",
                    "outcome": if call.class == CallClass::Rejected { "rejected" } else { "ok" },
                    "error": call.error,
                }));
            }
            for (position, inv) in call.invocations.iter().enumerate() {
                if inv.help {
                    help_pages.push(inv.page(command));
                }
                let rejected = culprit.flatten() == Some(position);
                let mut entry = json!({
                    "call": index,
                    "command": inv.text,
                    "kind": if inv.help { "help" } else { "command" },
                    "outcome": if rejected { "rejected" } else if inv.help { "help" } else { "ok" },
                });
                if rejected {
                    entry["error"] = json!(call.error);
                }
                commands.push(entry);
            }
        }
        json!({
            "calls": self.calls.len(),
            "help_reads": self.count(CallClass::Help),
            "rejected": self.count(CallClass::Rejected),
            "real": self.count(CallClass::Real),
            "other": self.count(CallClass::Other),
            "classes": self.calls.iter().map(|call| call.class.name()).collect::<Vec<_>>(),
            "commands": commands,
            "help_pages": help_pages,
        })
    }
}

/// Classify every `bash` call in an event stream.
///
/// Reads the shapes both subjects record: `tool.started` carries
/// `data.tool_call {id, name, arguments.commands}`, and `tool.completed`
/// carries `data.tool_call_id` with the output in `data.result` (content parts,
/// or the bash tool's `{stdout, stderr}` object) and/or `data.error`.
pub fn analyze(events: &[Value], surface: &Surface) -> Friction {
    let mut started: Vec<(Option<String>, String)> = Vec::new();
    let mut outputs: Vec<(Option<String>, String)> = Vec::new();
    for event in events {
        match event.get("type").and_then(Value::as_str) {
            Some("tool.started") => {
                let Some(call) = event.pointer("/data/tool_call") else {
                    continue;
                };
                if call.get("name").and_then(Value::as_str) != Some("bash") {
                    continue;
                }
                let script = call
                    .pointer("/arguments/commands")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let id = call.get("id").and_then(Value::as_str).map(str::to_string);
                started.push((id, script));
            }
            Some("tool.completed") => {
                let data = event.get("data").unwrap_or(&Value::Null);
                let name = data.get("tool_name").and_then(Value::as_str);
                if name.is_some_and(|name| name != "bash") {
                    continue;
                }
                let id = data
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                outputs.push((id, output_text(data)));
            }
            _ => {}
        }
    }

    // Pair by id where both sides carry one, otherwise in order.
    let mut unclaimed: Vec<Option<(Option<String>, String)>> =
        outputs.into_iter().map(Some).collect();
    let calls = started
        .into_iter()
        .map(|(id, script)| {
            let by_id = id.as_ref().and_then(|id| {
                unclaimed.iter().position(|slot| {
                    slot.as_ref()
                        .is_some_and(|(other, _)| other.as_ref() == Some(id))
                })
            });
            let slot = by_id.or_else(|| {
                unclaimed.iter().position(|slot| {
                    slot.as_ref()
                        .is_some_and(|(other, _)| other.is_none() || id.is_none())
                })
            });
            let output = slot
                .and_then(|index| unclaimed[index].take())
                .map(|(_, output)| output)
                .unwrap_or_default();
            classify(&script, &output, surface)
        })
        .collect();
    Friction { calls }
}

/// Classify one call from its script and the output the model saw.
pub fn classify(script: &str, output: &str, surface: &Surface) -> Call {
    let invocations = invocations(script, surface);
    let error = rejection(output, surface);
    let class = if error.is_some() {
        CallClass::Rejected
    } else if invocations.is_empty() {
        CallClass::Other
    } else if invocations.iter().all(|inv| inv.help) {
        CallClass::Help
    } else {
        CallClass::Real
    };
    Call {
        script: script.to_string(),
        output: output.to_string(),
        invocations,
        class,
        error,
    }
}

/// The refusal in `output`, from its first rejection line, when there is one.
fn rejection(output: &str, surface: &Surface) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.iter().position(|line| {
        let trimmed = line.trim_start();
        surface
            .rejection_prefixes
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
            || surface
                .rejection_markers
                .iter()
                .any(|marker| line.contains(marker))
    })?;
    // The refusal and the hint that follows it (clap's `tip:` and `Usage:`),
    // not the help page the tree may append after a blank line.
    let mut text = String::new();
    for line in lines[start..].iter().take(6) {
        if line.trim().is_empty() && !text.is_empty() {
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line.trim_end());
    }
    const LIMIT: usize = 600;
    if text.len() > LIMIT {
        let cut = (0..=LIMIT)
            .rev()
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(0);
        text.truncate(cut);
        text.push('…');
    }
    Some(text)
}

/// Every invocation of the surface's command in a script, in order.
pub fn invocations(script: &str, surface: &Surface) -> Vec<Invocation> {
    segments(script)
        .into_iter()
        .filter_map(|segment| {
            let mut words = words(&segment);
            // Leading shell keywords and `VAR=value` assignments are not the
            // command: `if everruns …`, `! tools …`, `FOO=1 everruns …`.
            let skip = words
                .iter()
                .take_while(|word| {
                    matches!(
                        word.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "do"
                            | "while"
                            | "until"
                            | "!"
                            | "{"
                            | "time"
                            | "exec"
                            | "command"
                    ) || is_assignment(word)
                })
                .count();
            words.drain(..skip);
            if words.first().map(String::as_str) != Some(surface.command) {
                return None;
            }
            words.remove(0);
            // Redirections end the command's own words.
            if let Some(end) = words.iter().position(|word| is_redirection(word)) {
                words.truncate(end);
            }
            let text = segment
                .trim()
                .trim_start_matches(|c: char| c == '{' || c == '!' || c.is_whitespace())
                .to_string();
            let text = text
                .find(surface.command)
                .map(|start| text[start..].trim().to_string())
                .unwrap_or(text);
            let help = is_help(&words, surface);
            Some(Invocation { text, words, help })
        })
        .collect()
}

fn is_help(words: &[String], surface: &Surface) -> bool {
    let Some(first) = words.first() else {
        return true;
    };
    if words.iter().any(|word| word == "--help" || word == "-h") || first == "help" {
        return true;
    }
    if surface.discovery_verbs.contains(&first.as_str()) {
        return true;
    }
    // A bare group: every word is a path word and together they name a node.
    !words.iter().any(|word| word.starts_with('-')) && (surface.is_group)(&words.join(" "))
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

fn is_redirection(word: &str) -> bool {
    let word = word.trim_start_matches(|c: char| c.is_ascii_digit());
    word.starts_with('>') || word.starts_with('<')
}

/// Split a script at command boundaries outside quotes: newlines, `;`, `|`,
/// `&`, parentheses and backticks. A heredoc body becomes segments too, which
/// is harmless: a JSON line does not start with the command's name.
fn segments(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in script.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (_, '\\') if quote != Some('\'') => {
                current.push(ch);
                escaped = true;
            }
            (Some(open), c) if c == open => {
                current.push(c);
                quote = None;
            }
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                current.push(ch);
                quote = Some(ch);
            }
            (None, '\n' | ';' | '|' | '&' | '(' | ')' | '`') => {
                if !current.trim().is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
            }
            (None, c) => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Split a segment into words, removing quotes.
fn words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for ch in segment.chars() {
        match (quote, ch) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

/// The text the model saw for a completed call.
fn output_text(data: &Value) -> String {
    let mut text = match data.get("result") {
        Some(Value::String(text)) => unwrap_shell_result(text),
        Some(Value::Array(parts)) => unwrap_shell_result(
            &parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Some(object @ Value::Object(_)) => {
            shell_streams(object).unwrap_or_else(|| object.to_string())
        }
        _ => String::new(),
    };
    if let Some(error) = data.get("error").and_then(Value::as_str) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(error);
    }
    text
}

/// The bash tool returns `{stdout, stderr, exit_code}`; when a transport
/// serialized that into text, read the streams back so a rejection line starts
/// a line again instead of sitting after an escaped `\n`.
fn unwrap_shell_result(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| shell_streams(&value))
        .unwrap_or_else(|| text.to_string())
}

fn shell_streams(value: &Value) -> Option<String> {
    let stdout = value.get("stdout").and_then(Value::as_str);
    let stderr = value.get("stderr").and_then(Value::as_str);
    if stdout.is_none() && stderr.is_none() {
        return None;
    }
    let mut text = stdout.unwrap_or_default().to_string();
    if let Some(stderr) = stderr.filter(|s| !s.is_empty()) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(stderr);
    }
    Some(text)
}

fn first_line(script: &str) -> String {
    script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

/// One run's friction, kept with what identifies it.
pub struct FrictionRun {
    pub case: String,
    pub target: String,
    pub trial: usize,
    pub passed: bool,
    pub tool_calls: usize,
    /// The run's error, so the tuning loop can drop runs that never reached a
    /// model (a missing key, a provider outage) instead of reading them as
    /// zero friction.
    pub error: Option<String>,
    pub friction: Friction,
}

/// Mean calls per run, split by what they were spent on, per case and overall.
///
/// This is the number the tuning loop drives down: models do not know the
/// surface from training, so every help read and rejected guess is a call
/// the help text could have saved. Runs with no `bash` calls carry no
/// breakdown and are left out of the friction columns.
pub fn summary_table(runs: &[FrictionRun]) -> String {
    let shell: Vec<&FrictionRun> = runs.iter().filter(|r| !r.friction.is_empty()).collect();
    if shell.is_empty() {
        return String::new();
    }
    let mut cases: std::collections::BTreeMap<(&str, &str), Vec<&FrictionRun>> =
        std::collections::BTreeMap::new();
    for run in &shell {
        cases
            .entry((run.case.as_str(), run.target.as_str()))
            .or_default()
            .push(run);
    }
    let mean = |group: &[&FrictionRun], pick: &dyn Fn(&FrictionRun) -> usize| -> f64 {
        group.iter().map(|r| pick(r) as f64).sum::<f64>() / group.len() as f64
    };
    let row = |label: &str, group: &[&FrictionRun]| -> String {
        format!(
            "{label:<56} {:>4} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2}  {}/{}\n",
            group.len(),
            mean(group, &|r| r.tool_calls),
            mean(group, &|r| r.friction.count(CallClass::Help)),
            mean(group, &|r| r.friction.count(CallClass::Rejected)),
            mean(group, &|r| r.friction.count(CallClass::Real)),
            mean(group, &|r| r.friction.count(CallClass::Other)),
            group.iter().filter(|r| r.passed).count(),
            group.len(),
        )
    };
    let mut out = format!(
        "\nfriction (mean per run)\n{:<56} {:>4} {:>6} {:>6} {:>6} {:>6} {:>6}  passed\n",
        "case", "runs", "calls", "help", "reject", "real", "other"
    );
    for ((case, target), group) in &cases {
        let label = if cases.keys().any(|(_, other)| other != target) {
            format!("{case}@{target}")
        } else {
            (*case).to_string()
        };
        out.push_str(&row(&label, group));
    }
    out.push_str(&row("overall", &shell));
    out
}

/// JSONL, one line per run: the input to the help-tuning loop.
pub fn report_jsonl(runs: &[FrictionRun], command: &str) -> String {
    let mut out = String::new();
    for run in runs {
        let mut line = run.friction.report(command);
        line["case"] = json!(run.case);
        line["target"] = json!(run.target);
        line["trial"] = json!(run.trial);
        line["passed"] = json!(run.passed);
        line["tool_calls"] = json!(run.tool_calls);
        line["error"] = json!(run.error);
        out.push_str(&line.to_string());
        out.push('\n');
    }
    out
}
