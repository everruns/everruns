//! The rules of a room, kept free of I/O so each one has a unit test:
//! handles, mentions, links, and when a message wakes an agent member.
//!
//! Loop limits (knowledge/runtime-resources/rooms.md, "Loop limits"):
//! - An agent is never woken by its own message.
//! - A message carries `hops`: 0 when a person or guest wrote it, one more
//!   than the message that woke the agent when an agent wrote it. Past
//!   `max_hops` no agent is woken, so two agents cannot talk forever.
//! - Each agent is woken at most `max_wakes_per_agent_hour` times an hour in a
//!   room, and a room takes at most `max_agent_posts_hour` agent posts an hour.
//!
//! A limit that stops a wake is recorded as a room notice, so the people in
//! the room see why an agent went quiet.
//!
//! An agent set to `relevant` wakes like `mentions`, and a message nobody
//! addressed to it is a candidate for a relevance check
//! ([`WakeDecision::Consider`], `crate::services::relevance`). Only messages a
//! person, guest or outside agent wrote are checked: an agent's post never
//! pulls another agent in by relevance, so agents cannot recruit each other
//! into a conversation. A room is checked at most
//! [`MAX_RELEVANCE_CHECKS_PER_ROOM_HOUR`] times an hour, and a check that a
//! loop limit would stop is skipped silently: nobody addressed that agent.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

/// Longest handle, in characters.
pub const MAX_HANDLE_CHARS: usize = 40;

/// Default loop limits. A room's `settings` may lower or raise them within
/// [`LoopLimits::clamped`].
pub const DEFAULT_MAX_HOPS: i32 = 6;
pub const DEFAULT_MAX_WAKES_PER_AGENT_HOUR: i64 = 30;
pub const DEFAULT_MAX_AGENT_POSTS_HOUR: i64 = 200;
/// Relevance checks of agent members in one room per hour, so a busy room
/// cannot spend without bound on checks.
pub const MAX_RELEVANCE_CHECKS_PER_ROOM_HOUR: i64 = 300;

/// When an agent member is woken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WakeMode {
    /// When mentioned, and on replies in threads it is in.
    #[default]
    Mentions,
    /// On every message it can see.
    Everything,
    /// When mentioned, on replies in threads it is in, and on messages a
    /// relevance check says are for it.
    Relevant,
    /// Never: the agent only reads.
    Never,
}

impl WakeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mentions => "mentions",
            Self::Everything => "everything",
            Self::Relevant => "relevant",
            Self::Never => "never",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "everything" => Self::Everything,
            "relevant" => Self::Relevant,
            "never" => Self::Never,
            _ => Self::Mentions,
        }
    }
}

/// Which conversation an agent member keeps in a room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionScope {
    /// One session per thread (a top-level message starts a thread).
    #[default]
    Thread,
    /// One session for the whole room.
    Room,
}

impl SessionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Room => "room",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "room" => Self::Room,
            _ => Self::Thread,
        }
    }
}

/// Loop limits in force in one room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LoopLimits {
    /// Agent posts in a row before agents stop being woken.
    pub max_hops: i32,
    /// Wakes of one agent in one room per hour.
    pub max_wakes_per_agent_hour: i64,
    /// Agent posts in one room per hour.
    pub max_agent_posts_hour: i64,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_hops: DEFAULT_MAX_HOPS,
            max_wakes_per_agent_hour: DEFAULT_MAX_WAKES_PER_AGENT_HOUR,
            max_agent_posts_hour: DEFAULT_MAX_AGENT_POSTS_HOUR,
        }
    }
}

impl LoopLimits {
    /// The limits a room's `settings` ask for, kept within bounds a room
    /// cannot leave: at least one hop, at most 20; at most 120 wakes and 1000
    /// agent posts an hour.
    pub fn from_settings(settings: &serde_json::Value) -> Self {
        let defaults = Self::default();
        let limits = settings.get("loop_limits");
        let int = |key: &str| limits.and_then(|v| v.get(key)).and_then(|v| v.as_i64());
        Self {
            max_hops: int("max_hops").map_or(defaults.max_hops, |v| v as i32),
            max_wakes_per_agent_hour: int("max_wakes_per_agent_hour")
                .unwrap_or(defaults.max_wakes_per_agent_hour),
            max_agent_posts_hour: int("max_agent_posts_hour")
                .unwrap_or(defaults.max_agent_posts_hour),
        }
        .clamped()
    }

    pub fn clamped(self) -> Self {
        Self {
            max_hops: self.max_hops.clamp(1, 20),
            max_wakes_per_agent_hour: self.max_wakes_per_agent_hour.clamp(1, 120),
            max_agent_posts_hour: self.max_agent_posts_hour.clamp(1, 1000),
        }
    }
}

/// A handle derived from a display name: lowercase letters, digits, `_`, `-`
/// and `.`, starting with a letter or digit. `None` when nothing usable is
/// left.
pub fn handle_from_name(name: &str) -> Option<String> {
    let mut handle = String::new();
    let mut last_dash = false;
    for ch in name.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            handle.push(ch);
            last_dash = false;
        } else if matches!(ch, '_' | '.') {
            if !handle.is_empty() {
                handle.push(ch);
                last_dash = false;
            }
        } else if !handle.is_empty() && !last_dash {
            handle.push('-');
            last_dash = true;
        }
        if handle.chars().count() >= MAX_HANDLE_CHARS {
            break;
        }
    }
    let handle = handle.trim_end_matches(['-', '_', '.']).to_string();
    (!handle.is_empty()).then_some(handle)
}

/// Whether a handle is valid as given (what the database enforces).
pub fn is_valid_handle(handle: &str) -> bool {
    let mut chars = handle.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && handle.chars().count() <= MAX_HANDLE_CHARS
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c))
}

/// A free handle: `base`, or `base-2`, `base-3`, ... when taken.
pub fn unique_handle(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let keep = MAX_HANDLE_CHARS.saturating_sub(suffix.len());
            format!("{}{suffix}", base.chars().take(keep).collect::<String>())
        })
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded range always finds a free handle")
}

/// The handles `text` mentions as `@handle`, in order, without repeats. A
/// mention starts at the beginning of the text or after a character that
/// cannot be part of a word or an email address, so `a@b.co` is not one.
/// Trailing punctuation is not part of the handle (`@ana.` names `ana`).
pub fn mentioned_handles(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let starts = chars[i] == '@'
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || "_.-@".contains(chars[i - 1])));
        if !starts {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len()
            && (chars[j].is_ascii_alphanumeric() || "_.-".contains(chars[j]))
            && j - i <= MAX_HANDLE_CHARS
        {
            j += 1;
        }
        let handle: String = chars[i + 1..j]
            .iter()
            .collect::<String>()
            .trim_end_matches(['.', '-', '_'])
            .to_ascii_lowercase();
        if !handle.is_empty() && !found.contains(&handle) {
            found.push(handle);
        }
        i = j.max(i + 1);
    }
    found
}

/// A message's link in the builder's product: `{room_id}`, `{message_id}`
/// and `{thread_id}` filled in (the thread is the message itself at the top
/// level). `None` without a template.
pub fn render_link(
    template: Option<&str>,
    room_id: &str,
    message_id: &str,
    thread_id: Option<&str>,
) -> Option<String> {
    let template = template?.trim();
    if template.is_empty() {
        return None;
    }
    Some(
        template
            .replace("{room_id}", room_id)
            .replace("{message_id}", message_id)
            .replace("{thread_id}", thread_id.unwrap_or(message_id)),
    )
}

/// Whether a link template is acceptable: an `https` URL (or `http` to
/// localhost, for development) of a sane length.
pub fn is_valid_link_template(template: &str) -> bool {
    let template = template.trim();
    template.len() <= 2_000
        && (template.starts_with("https://")
            || template.starts_with("http://localhost")
            || template.starts_with("http://127.0.0.1"))
}

/// What the room knows about a message when deciding whom it wakes.
#[derive(Debug, Clone, Copy)]
pub struct WakeContext {
    /// Member who wrote it; `None` for a room notice.
    pub author: Option<uuid::Uuid>,
    /// Agent posts in a row up to and including this message.
    pub hops: i32,
    /// Agent posts in this room in the last hour.
    pub room_agent_posts_last_hour: i64,
    /// Relevance checks in this room in the last hour.
    pub room_checks_last_hour: i64,
}

/// One agent member, as the wake decision sees it.
#[derive(Debug, Clone, Copy)]
pub struct WakeCandidate {
    pub member: uuid::Uuid,
    pub mode: WakeMode,
    /// The message mentions this member.
    pub mentioned: bool,
    /// The message is a reply in a thread this member started, replied in, or
    /// was mentioned in.
    pub in_thread: bool,
    /// Times this member was woken in this room in the last hour.
    pub wakes_last_hour: i64,
}

/// Why an agent member is or is not woken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeDecision {
    Wake,
    /// Nobody addressed it, and it wakes on relevant messages: ask the
    /// relevance check, then wake it when the check says so.
    Consider,
    /// Not addressed to it: nothing to say about it.
    NotAddressed,
    /// The agent wrote the message.
    OwnMessage,
    /// Its wake setting is `never`.
    Muted,
    /// A loop limit stopped it; the room is told.
    Limited(LimitHit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitHit {
    Hops,
    AgentHourly,
    RoomHourly,
}

impl LimitHit {
    /// The room notice for this limit.
    pub fn notice(self, handle: &str, limits: &LoopLimits) -> String {
        match self {
            Self::Hops => format!(
                "@{handle} was not woken: {} agent messages in a row. A person can reply to continue.",
                limits.max_hops
            ),
            Self::AgentHourly => format!(
                "@{handle} was not woken: it reached {} wakes in this room this hour.",
                limits.max_wakes_per_agent_hour
            ),
            Self::RoomHourly => format!(
                "Agents were not woken: this room reached {} agent messages this hour.",
                limits.max_agent_posts_hour
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hops => "hops",
            Self::AgentHourly => "agent_hourly",
            Self::RoomHourly => "room_hourly",
        }
    }
}

/// Whether a message wakes one agent member.
pub fn wake_decision(
    message: &WakeContext,
    candidate: &WakeCandidate,
    limits: &LoopLimits,
) -> WakeDecision {
    if message.author == Some(candidate.member) {
        return WakeDecision::OwnMessage;
    }
    let addressed = match candidate.mode {
        WakeMode::Never => return WakeDecision::Muted,
        WakeMode::Everything => true,
        WakeMode::Mentions | WakeMode::Relevant => candidate.mentioned || candidate.in_thread,
    };
    if !addressed {
        let considered = candidate.mode == WakeMode::Relevant
            && message.author.is_some()
            && message.hops == 0
            && message.room_checks_last_hour < MAX_RELEVANCE_CHECKS_PER_ROOM_HOUR
            && limit_hit(message, candidate, limits).is_none();
        return if considered {
            WakeDecision::Consider
        } else {
            WakeDecision::NotAddressed
        };
    }
    match limit_hit(message, candidate, limits) {
        Some(hit) => WakeDecision::Limited(hit),
        None => WakeDecision::Wake,
    }
}

fn limit_hit(
    message: &WakeContext,
    candidate: &WakeCandidate,
    limits: &LoopLimits,
) -> Option<LimitHit> {
    if message.hops >= limits.max_hops {
        Some(LimitHit::Hops)
    } else if message.room_agent_posts_last_hour >= limits.max_agent_posts_hour {
        Some(LimitHit::RoomHourly)
    } else if candidate.wakes_last_hour >= limits.max_wakes_per_agent_hour {
        Some(LimitHit::AgentHourly)
    } else {
        None
    }
}

/// The hops of a message an agent posts after being woken by a message with
/// `woken_by_hops`.
pub fn agent_post_hops(woken_by_hops: i32) -> i32 {
    woken_by_hops.saturating_add(1)
}

/// Longest room file path, in characters.
pub const MAX_FILE_PATH_CHARS: usize = 512;

/// A room file path as stored: from the folder's root, with a leading `/`.
/// `notes.md` and `/notes.md` are the same file. Rejects `.`, `..`, empty
/// segments and control characters.
pub fn clean_file_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let path = trimmed.trim_end_matches('/');
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    if path == "/" {
        return Err("path names no file".into());
    }
    if path.chars().count() > MAX_FILE_PATH_CHARS {
        return Err(format!(
            "path is longer than {MAX_FILE_PATH_CHARS} characters"
        ));
    }
    if path.chars().any(char::is_control) {
        return Err("path has a control character".into());
    }
    if path[1..]
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err("path has an empty, '.' or '..' segment".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn handles_come_from_names() {
        assert_eq!(handle_from_name("Ana Lopez").as_deref(), Some("ana-lopez"));
        assert_eq!(
            handle_from_name("  Release Bot!! ").as_deref(),
            Some("release-bot")
        );
        assert_eq!(
            handle_from_name("o'Brien.dev").as_deref(),
            Some("o-brien.dev")
        );
        assert_eq!(handle_from_name("Їжак"), None);
        assert_eq!(handle_from_name("___"), None);
        let long = handle_from_name(&"a".repeat(100)).unwrap();
        assert_eq!(long.len(), MAX_HANDLE_CHARS);
        assert!(is_valid_handle(&long));
    }

    #[test]
    fn handle_validity_matches_the_database_rule() {
        for ok in ["ana", "a1", "release-bot", "x.y_z", "9lives"] {
            assert!(is_valid_handle(ok), "{ok}");
        }
        for bad in ["", "-ana", "Ana", "a b", "ana!", &"a".repeat(41)] {
            assert!(!is_valid_handle(bad), "{bad}");
        }
    }

    #[test]
    fn taken_handles_get_a_number() {
        let taken: HashSet<String> = ["ana".into(), "ana-2".into()].into();
        assert_eq!(unique_handle("ana", &taken), "ana-3");
        assert_eq!(unique_handle("bo", &taken), "bo");
        let base = "b".repeat(MAX_HANDLE_CHARS);
        let taken: HashSet<String> = [base.clone()].into();
        let next = unique_handle(&base, &taken);
        assert!(is_valid_handle(&next) && next.ends_with("-2"));
    }

    #[test]
    fn mentions_are_found_once_in_order() {
        assert_eq!(
            mentioned_handles("@ana can you ask @release-bot? cc @ana."),
            vec!["ana", "release-bot"]
        );
        assert_eq!(mentioned_handles("(@Ana) and @bo_1,"), vec!["ana", "bo_1"]);
    }

    #[test]
    fn emails_and_bare_signs_are_not_mentions() {
        assert!(mentioned_handles("mail ana@example.com now").is_empty());
        assert!(mentioned_handles("@ @@ x@").is_empty());
    }

    #[test]
    fn links_fill_their_template() {
        let link = render_link(
            Some("https://app.example.com/r/{room_id}/t/{thread_id}#{message_id}"),
            "room_1",
            "rmsg_2",
            Some("rmsg_1"),
        );
        assert_eq!(
            link.as_deref(),
            Some("https://app.example.com/r/room_1/t/rmsg_1#rmsg_2")
        );
        let top = render_link(Some("https://x.test/{thread_id}"), "r", "m", None);
        assert_eq!(top.as_deref(), Some("https://x.test/m"));
        assert_eq!(render_link(None, "r", "m", None), None);
        assert_eq!(render_link(Some("  "), "r", "m", None), None);
    }

    #[test]
    fn link_templates_must_be_https() {
        assert!(is_valid_link_template("https://x.test/{room_id}"));
        assert!(is_valid_link_template("http://localhost:3000/{room_id}"));
        assert!(!is_valid_link_template("http://x.test/{room_id}"));
        assert!(!is_valid_link_template("javascript:alert(1)"));
    }

    fn candidate(mode: WakeMode) -> WakeCandidate {
        WakeCandidate {
            member: Uuid::from_u128(2),
            mode,
            mentioned: false,
            in_thread: false,
            wakes_last_hour: 0,
        }
    }

    fn person_message() -> WakeContext {
        WakeContext {
            author: Some(Uuid::from_u128(1)),
            hops: 0,
            room_agent_posts_last_hour: 0,
            room_checks_last_hour: 0,
        }
    }

    #[test]
    fn relevant_wakes_like_mentions_and_considers_the_rest() {
        let limits = LoopLimits::default();
        let msg = person_message();
        let relevant = candidate(WakeMode::Relevant);
        assert_eq!(
            wake_decision(&msg, &relevant, &limits),
            WakeDecision::Consider
        );
        let mentioned = WakeCandidate {
            mentioned: true,
            ..relevant
        };
        assert_eq!(wake_decision(&msg, &mentioned, &limits), WakeDecision::Wake);
        let threaded = WakeCandidate {
            in_thread: true,
            ..relevant
        };
        assert_eq!(wake_decision(&msg, &threaded, &limits), WakeDecision::Wake);
    }

    #[test]
    fn only_messages_people_write_are_checked() {
        let limits = LoopLimits::default();
        let relevant = candidate(WakeMode::Relevant);
        // An agent's post never pulls another agent in by relevance.
        let agent_post = WakeContext {
            hops: 1,
            ..person_message()
        };
        assert_eq!(
            wake_decision(&agent_post, &relevant, &limits),
            WakeDecision::NotAddressed
        );
        // Room notices are nobody's message.
        let notice = WakeContext {
            author: None,
            ..person_message()
        };
        assert_eq!(
            wake_decision(&notice, &relevant, &limits),
            WakeDecision::NotAddressed
        );
    }

    #[test]
    fn checks_are_capped_and_limits_skip_them_silently() {
        let limits = LoopLimits::default();
        let relevant = candidate(WakeMode::Relevant);
        let busy = WakeContext {
            room_checks_last_hour: MAX_RELEVANCE_CHECKS_PER_ROOM_HOUR,
            ..person_message()
        };
        assert_eq!(
            wake_decision(&busy, &relevant, &limits),
            WakeDecision::NotAddressed
        );
        let tired = WakeCandidate {
            wakes_last_hour: limits.max_wakes_per_agent_hour,
            ..relevant
        };
        assert_eq!(
            wake_decision(&person_message(), &tired, &limits),
            WakeDecision::NotAddressed
        );
    }

    #[test]
    fn mention_mode_wakes_on_mentions_and_threads_only() {
        let limits = LoopLimits::default();
        let msg = person_message();
        assert_eq!(
            wake_decision(&msg, &candidate(WakeMode::Mentions), &limits),
            WakeDecision::NotAddressed
        );
        let mentioned = WakeCandidate {
            mentioned: true,
            ..candidate(WakeMode::Mentions)
        };
        assert_eq!(wake_decision(&msg, &mentioned, &limits), WakeDecision::Wake);
        let threaded = WakeCandidate {
            in_thread: true,
            ..candidate(WakeMode::Mentions)
        };
        assert_eq!(wake_decision(&msg, &threaded, &limits), WakeDecision::Wake);
    }

    #[test]
    fn everything_wakes_and_never_does_not() {
        let limits = LoopLimits::default();
        let msg = person_message();
        assert_eq!(
            wake_decision(&msg, &candidate(WakeMode::Everything), &limits),
            WakeDecision::Wake
        );
        let muted = WakeCandidate {
            mentioned: true,
            ..candidate(WakeMode::Never)
        };
        assert_eq!(wake_decision(&msg, &muted, &limits), WakeDecision::Muted);
    }

    #[test]
    fn an_agent_is_never_woken_by_itself() {
        let limits = LoopLimits::default();
        let own = WakeContext {
            author: Some(Uuid::from_u128(2)),
            ..person_message()
        };
        assert_eq!(
            wake_decision(&own, &candidate(WakeMode::Everything), &limits),
            WakeDecision::OwnMessage
        );
    }

    #[test]
    fn loop_limits_stop_wakes() {
        let limits = LoopLimits::default();
        let everything = candidate(WakeMode::Everything);
        let deep = WakeContext {
            hops: limits.max_hops,
            ..person_message()
        };
        assert_eq!(
            wake_decision(&deep, &everything, &limits),
            WakeDecision::Limited(LimitHit::Hops)
        );
        let busy_room = WakeContext {
            room_agent_posts_last_hour: limits.max_agent_posts_hour,
            ..person_message()
        };
        assert_eq!(
            wake_decision(&busy_room, &everything, &limits),
            WakeDecision::Limited(LimitHit::RoomHourly)
        );
        let tired = WakeCandidate {
            wakes_last_hour: limits.max_wakes_per_agent_hour,
            ..everything
        };
        assert_eq!(
            wake_decision(&person_message(), &tired, &limits),
            WakeDecision::Limited(LimitHit::AgentHourly)
        );
        // Not addressed beats a limit: nobody is told about a wake that was
        // never going to happen.
        assert_eq!(
            wake_decision(&deep, &candidate(WakeMode::Mentions), &limits),
            WakeDecision::NotAddressed
        );
    }

    #[test]
    fn two_agents_stop_after_the_hop_limit() {
        // A person writes (hops 0); agents A and B answer each other.
        let limits = LoopLimits::default();
        let mut hops = 0;
        let mut posts = 0;
        loop {
            let msg = WakeContext {
                author: Some(Uuid::from_u128(if posts % 2 == 0 { 1 } else { 2 })),
                hops,
                room_agent_posts_last_hour: posts,
                room_checks_last_hour: 0,
            };
            let next = WakeCandidate {
                member: Uuid::from_u128(if posts % 2 == 0 { 2 } else { 1 }),
                ..candidate(WakeMode::Everything)
            };
            if wake_decision(&msg, &next, &limits) != WakeDecision::Wake {
                break;
            }
            hops = agent_post_hops(hops);
            posts += 1;
        }
        assert_eq!(posts, i64::from(limits.max_hops));
    }

    #[test]
    fn room_settings_adjust_limits_within_bounds() {
        let limits = LoopLimits::from_settings(&serde_json::json!({
            "loop_limits": {"max_hops": 2, "max_wakes_per_agent_hour": 9999}
        }));
        assert_eq!(limits.max_hops, 2);
        assert_eq!(limits.max_wakes_per_agent_hour, 120);
        assert_eq!(limits.max_agent_posts_hour, DEFAULT_MAX_AGENT_POSTS_HOUR);
        let zero = LoopLimits::from_settings(&serde_json::json!({"loop_limits": {"max_hops": 0}}));
        assert_eq!(zero.max_hops, 1);
        assert_eq!(
            LoopLimits::from_settings(&serde_json::json!({})),
            LoopLimits::default()
        );
    }

    #[test]
    fn modes_round_trip() {
        for mode in [
            WakeMode::Mentions,
            WakeMode::Everything,
            WakeMode::Relevant,
            WakeMode::Never,
        ] {
            assert_eq!(WakeMode::parse(mode.as_str()), mode);
        }
        for scope in [SessionScope::Thread, SessionScope::Room] {
            assert_eq!(SessionScope::parse(scope.as_str()), scope);
        }
    }

    #[test]
    fn file_paths_are_rooted_and_cannot_escape() {
        assert_eq!(clean_file_path("notes.md").unwrap(), "/notes.md");
        assert_eq!(
            clean_file_path("/plans/launch.md").unwrap(),
            "/plans/launch.md"
        );
        assert_eq!(clean_file_path("/plans/").unwrap(), "/plans");
        assert!(clean_file_path("/").is_err());
        assert!(clean_file_path("../etc/passwd").is_err());
        assert!(clean_file_path("/a//b").is_err());
        assert!(clean_file_path("/a/./b").is_err());
        assert!(clean_file_path("/a b.md").is_ok());
        assert!(clean_file_path("/a\u{0}b").is_err());
    }
}
