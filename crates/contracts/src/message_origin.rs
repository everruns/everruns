// Message origin: who wrote a user-role message when no person did.
//
// Design (knowledge/execution/message-origin.md):
// - One reserved, platform-only metadata key, `everruns_origin`, names the
//   writer of every user-role message the platform injects: a schedule run, an
//   agent trigger, a channel schedule or webhook, a thread wake-up, or a
//   platform notice. No origin means a person (or their client) wrote it.
// - It answers "who wrote these words". The event's `initiator` answers "whose
//   authority is this", which approvals, payments and audit use; the two stay
//   separate.
// - `name` is the display name at write time, so renaming or deleting the
//   schedule never rewrites history.
// - Older rows stay as written. The reader maps the earlier markers (the
//   `"task_wake"` string, and `source: "schedule" | "agent_trigger" |
//   "app_schedule" | "app_webhook"`) to the same shape, so nothing is
//   backfilled.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Message metadata key naming where a user-role message came from when no
/// person wrote it. Reserved: client-supplied metadata loses this key, so only
/// the platform can set it.
pub const MESSAGE_ORIGIN_METADATA_KEY: &str = "everruns_origin";
/// Legacy string value of [`MESSAGE_ORIGIN_METADATA_KEY`] for a task wake-up.
pub const MESSAGE_ORIGIN_TASK_WAKE: &str = "task_wake";
/// Metadata key the schedule limits write next to a scheduled message.
const SCHEDULE_RUN_METADATA_KEY: &str = "schedule_run";
/// Model-facing label on thread wake-ups.
const TASK_WAKE_HEADER: &str = "[Automatic task update, not written by the person] ";
/// Longest display name derived from free text, in characters.
const NAME_MAX_CHARS: usize = 60;

/// What wrote a platform-injected message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageOriginKind {
    /// A session schedule fired (an agent's own schedule, or a continuation
    /// scheduled after a usage limit reset).
    Schedule,
    /// An agent trigger fired (schedule, GitHub, MCP event).
    AgentTrigger,
    /// A channel's schedule fired.
    ChannelSchedule,
    /// A webhook call arrived on a channel.
    Webhook,
    /// The platform woke a coordinator about its threads.
    TaskWake,
    /// A platform notice, such as a cancellation.
    System,
    /// Someone reacted to a message while the agent was working, or the
    /// agent asked to be woken by reactions. The message text is the
    /// rendered reaction note (`message_reactions::ReactionNote`).
    Reaction,
}

impl MessageOriginKind {
    /// Wire name, as stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::AgentTrigger => "agent_trigger",
            Self::ChannelSchedule => "channel_schedule",
            Self::Webhook => "webhook",
            Self::TaskWake => "task_wake",
            Self::System => "system",
            Self::Reaction => "reaction",
        }
    }

    /// Short label a person or the model reads, such as "Scheduled".
    pub fn label(self) -> &'static str {
        match self {
            Self::Schedule | Self::ChannelSchedule => "Scheduled",
            Self::AgentTrigger => "Trigger",
            Self::Webhook => "Webhook",
            Self::TaskWake => "Automatic update",
            Self::System => "System",
            Self::Reaction => "Reaction",
        }
    }
}

/// Run counter and limits of a scheduled message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageOriginRun {
    /// Which run this is, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// The most runs the schedule may make.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// When the schedule stops at the latest (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<String>,
    /// The condition under which the agent should stop the schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_when: Option<String>,
}

impl MessageOriginRun {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    fn from_value(value: &Value) -> Option<Self> {
        let count = |key: &str| value.get(key).and_then(Value::as_u64);
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        };
        let run = Self {
            number: count("number"),
            limit: count("limit"),
            ends_at: text("ends_at"),
            stop_when: text("stop_when"),
        };
        (!run.is_empty()).then_some(run)
    }
}

/// Who wrote a platform-injected user-role message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageOrigin {
    pub kind: MessageOriginKind,
    /// Id of the schedule, trigger or channel that wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Display name at write time, such as "Shipmail claim poll".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Run counter and limits, for schedules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<MessageOriginRun>,
}

impl MessageOrigin {
    pub fn new(kind: MessageOriginKind) -> Self {
        Self {
            kind,
            source_id: None,
            name: None,
            run: None,
        }
    }

    pub fn with_source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    /// Set the display name, trimmed to its first line and a readable length.
    pub fn with_name(mut self, name: impl AsRef<str>) -> Self {
        self.name = display_name(name.as_ref());
        self
    }

    pub fn with_run(mut self, run: MessageOriginRun) -> Self {
        self.run = (!run.is_empty()).then_some(run);
        self
    }

    /// A session schedule run. The schedule has no name of its own, so the
    /// first sentence of its prompt is its name.
    pub fn schedule(schedule_id: impl Into<String>, prompt: &str) -> Self {
        Self::new(MessageOriginKind::Schedule)
            .with_source_id(schedule_id)
            .with_name(prompt)
    }

    /// A platform notice, such as "Work cancelled".
    pub fn system(name: &str) -> Self {
        Self::new(MessageOriginKind::System).with_name(name)
    }

    /// The stored metadata value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Write this origin into message metadata.
    pub fn write(&self, metadata: &mut Option<HashMap<String, Value>>) {
        metadata
            .get_or_insert_default()
            .insert(MESSAGE_ORIGIN_METADATA_KEY.to_string(), self.to_value());
    }

    /// Metadata holding only this origin.
    pub fn into_metadata(self) -> HashMap<String, Value> {
        let mut metadata = None;
        self.write(&mut metadata);
        metadata.unwrap_or_default()
    }

    /// Read the origin from message metadata, including the earlier markers.
    pub fn from_metadata(metadata: &serde_json::Map<String, Value>) -> Option<Self> {
        let get = |key: &str| metadata.get(key);
        let text = |key: &str| get(key).and_then(Value::as_str).map(str::to_string);
        let mut origin = match get(MESSAGE_ORIGIN_METADATA_KEY) {
            Some(Value::Object(_)) => {
                serde_json::from_value::<Self>(get(MESSAGE_ORIGIN_METADATA_KEY)?.clone()).ok()?
            }
            Some(Value::String(kind)) if kind == MESSAGE_ORIGIN_TASK_WAKE => {
                Self::new(MessageOriginKind::TaskWake)
            }
            _ => {
                let (kind, id_key) = match get("source").and_then(Value::as_str)? {
                    "schedule" => (MessageOriginKind::Schedule, "schedule_id"),
                    "agent_trigger" => (MessageOriginKind::AgentTrigger, "agent_trigger_id"),
                    "app_schedule" => (MessageOriginKind::ChannelSchedule, "app_channel_id"),
                    "app_webhook" => (MessageOriginKind::Webhook, "app_channel_id"),
                    _ => return None,
                };
                let mut origin = Self::new(kind);
                origin.source_id = text(id_key);
                origin
            }
        };
        if origin.run.is_none()
            && let Some(run) = get(SCHEDULE_RUN_METADATA_KEY).and_then(MessageOriginRun::from_value)
        {
            origin.run = Some(run);
        }
        Some(origin)
    }

    /// Read the origin from a message's metadata map.
    pub fn from_message_metadata(metadata: Option<&HashMap<String, Value>>) -> Option<Self> {
        let metadata = metadata?;
        // Only the keys the reader looks at, so a large metadata map is not
        // cloned whole.
        let map = [
            MESSAGE_ORIGIN_METADATA_KEY,
            "source",
            "schedule_id",
            "agent_trigger_id",
            "app_channel_id",
            SCHEDULE_RUN_METADATA_KEY,
        ]
        .into_iter()
        .filter_map(|key| metadata.get(key).map(|v| (key.to_string(), v.clone())))
        .collect();
        Self::from_metadata(&map)
    }

    /// The one line the model reads before the message text, so a platform
    /// message is never mistaken for the person's own words. Built only from
    /// the stored origin, so the same message renders the same line on every
    /// replay and the prompt cache holds.
    pub fn model_header(&self) -> String {
        let mut line = match self.kind {
            MessageOriginKind::Schedule | MessageOriginKind::ChannelSchedule => {
                match self.run.as_ref().and_then(|run| run.number) {
                    Some(number) => match self.run.as_ref().and_then(|run| run.limit) {
                        Some(limit) => format!("Scheduled run {number} of {limit}"),
                        None => format!("Scheduled run {number}"),
                    },
                    None => "Scheduled run".to_string(),
                }
            }
            // Kept word for word from before origins existed, so coordinator
            // sessions keep their cached prompt prefix.
            MessageOriginKind::TaskWake => return TASK_WAKE_HEADER.to_string(),
            // The text is the reaction note itself; it carries its own label.
            MessageOriginKind::Reaction => return String::new(),
            kind => format!("{} message", kind.label()),
        };
        if let Some(name) = &self.name
            && self.kind != MessageOriginKind::System
        {
            line.push_str(&format!(": \"{name}\""));
        }
        line.push('.');
        if let Some(run) = &self.run {
            if let Some(stop_when) = &run.stop_when {
                line.push_str(&format!(" Stop when: {stop_when}."));
            }
            if let Some(ends_at) = &run.ends_at {
                line.push_str(&format!(" Ends at {ends_at}."));
            }
        }
        line.push_str(" Not written by the person.");
        format!("[{line}] ")
    }
}

/// First line of free text, cut at the first sentence end and a readable
/// length, for use as a display name.
fn display_name(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let sentence = line
        .find(". ")
        .map_or(line, |end| &line[..end])
        .trim_end_matches('.');
    let mut name: String = sentence.chars().take(NAME_MAX_CHARS).collect();
    if sentence.chars().count() > NAME_MAX_CHARS {
        name = name.trim_end().to_string();
        name.push('…');
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_schedule_origin_round_trips_and_names_itself_from_the_prompt() {
        let origin = MessageOrigin::schedule(
            "sched_01",
            "Shipmail claim poll for Everruns Worker agent. Read /workspace/state.json.",
        )
        .with_run(MessageOriginRun {
            number: Some(12),
            limit: Some(96),
            ends_at: None,
            stop_when: Some("claim approved".into()),
        });
        assert_eq!(
            origin.name.as_deref(),
            Some("Shipmail claim poll for Everruns Worker agent")
        );
        let metadata = origin.clone().into_metadata();
        assert_eq!(
            MessageOrigin::from_message_metadata(Some(&metadata)),
            Some(origin)
        );
    }

    #[test]
    fn a_long_first_line_is_cut_with_an_ellipsis() {
        let name = display_name(&"x".repeat(200)).unwrap();
        assert_eq!(name.chars().count(), NAME_MAX_CHARS + 1);
        assert!(name.ends_with('…'));
        assert_eq!(display_name("  \n\n"), None);
    }

    #[test]
    fn earlier_markers_read_as_the_same_origin() {
        assert_eq!(
            MessageOrigin::from_metadata(&map(json!({"everruns_origin": "task_wake"}))),
            Some(MessageOrigin::new(MessageOriginKind::TaskWake))
        );
        let schedule = MessageOrigin::from_metadata(&map(
            json!({"source": "schedule", "schedule_id": "sched_9"}),
        ))
        .unwrap();
        assert_eq!(schedule.kind, MessageOriginKind::Schedule);
        assert_eq!(schedule.source_id.as_deref(), Some("sched_9"));
        let trigger = MessageOrigin::from_metadata(&map(
            json!({"source": "agent_trigger", "agent_trigger_id": "trg_1"}),
        ))
        .unwrap();
        assert_eq!(trigger.kind, MessageOriginKind::AgentTrigger);
        assert_eq!(trigger.source_id.as_deref(), Some("trg_1"));
        assert_eq!(
            MessageOrigin::from_metadata(&map(json!({"source": "app_webhook"})))
                .unwrap()
                .kind,
            MessageOriginKind::Webhook
        );
        assert_eq!(
            MessageOrigin::from_metadata(&map(json!({"source": "app_schedule"})))
                .unwrap()
                .kind,
            MessageOriginKind::ChannelSchedule
        );
    }

    #[test]
    fn messages_a_person_sent_have_no_origin() {
        for metadata in [
            json!({}),
            json!({"source": "voice"}),
            json!({"source": "fcp"}),
            json!({"everruns_origin": "made_up"}),
            json!({"everruns_origin": {"kind": "made_up"}}),
        ] {
            assert_eq!(MessageOrigin::from_metadata(&map(metadata)), None);
        }
        assert_eq!(MessageOrigin::from_message_metadata(None), None);
    }

    #[test]
    fn the_schedule_limits_run_counter_fills_the_run() {
        let origin = MessageOrigin::from_metadata(&map(json!({
            "source": "schedule",
            "schedule_id": "sched_1",
            "schedule_run": {"number": 3, "limit": 96, "stop_when": "done", "budget": 2.0}
        })))
        .unwrap();
        assert_eq!(
            origin.run,
            Some(MessageOriginRun {
                number: Some(3),
                limit: Some(96),
                ends_at: None,
                stop_when: Some("done".into()),
            })
        );
    }

    #[test]
    fn the_model_header_says_who_wrote_it() {
        let origin =
            MessageOrigin::schedule("sched_1", "Shipmail claim poll").with_run(MessageOriginRun {
                number: Some(12),
                limit: Some(96),
                ends_at: Some("2026-11-02T03:25:00Z".into()),
                stop_when: Some("claim approved".into()),
            });
        assert_eq!(
            origin.model_header(),
            "[Scheduled run 12 of 96: \"Shipmail claim poll\". Stop when: claim approved. \
             Ends at 2026-11-02T03:25:00Z. Not written by the person.] "
        );
        assert_eq!(
            MessageOrigin::new(MessageOriginKind::TaskWake).model_header(),
            "[Automatic task update, not written by the person] "
        );
        assert_eq!(
            MessageOrigin::system("Work cancelled").model_header(),
            "[System message. Not written by the person.] "
        );
        assert_eq!(
            MessageOrigin::new(MessageOriginKind::AgentTrigger)
                .with_name("GitHub")
                .model_header(),
            "[Trigger message: \"GitHub\". Not written by the person.] "
        );
    }
}
