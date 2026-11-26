use super::super::agent_channels::record::work_chat::{WorkChatConfig, WorkChatMembership};
use crate::domains::common::CommandError;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::ApplicationId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Audience admission and optional workspace features.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationConfig {
    /// Who may sign in: organization members by default, invited people, or anyone.
    #[serde(default = "default_membership")]
    pub membership: WorkChatMembership,
    /// Verified email addresses admitted when membership is invites.
    #[serde(default)]
    pub invites: Vec<String>,
    /// Show a launcher for published Applications in the organization sidebar.
    #[serde(default)]
    pub show_in_sidebar: bool,
    /// Reserved until the automation runtime is available. Enabling is rejected.
    #[serde(default)]
    pub automations: bool,
}
fn default_membership() -> WorkChatMembership {
    WorkChatMembership::OrgUsers
}
impl Default for ApplicationConfig {
    fn default() -> Self {
        Self {
            membership: default_membership(),
            invites: Vec::new(),
            show_in_sidebar: false,
            automations: false,
        }
    }
}
impl ApplicationConfig {
    #[allow(clippy::field_reassign_with_default)] // WorkChatConfig has private legacy fields.
    pub fn audience(&self) -> WorkChatConfig {
        let mut config = WorkChatConfig::default();
        config.membership = self.membership;
        config.invites = self.invites.clone();
        config
    }
    pub fn prepare(&mut self) -> Result<(), CommandError> {
        if self.automations {
            return Err(CommandError::bad_request(
                "Automations are not available yet",
            ));
        }
        if self.invites.len() > 50 {
            return Err(CommandError::bad_request(
                "At most 50 invited email addresses are supported",
            ));
        }
        self.invites = self
            .invites
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        self.invites.sort();
        self.invites.dedup();
        if self
            .invites
            .iter()
            .any(|s| s.len() > 254 || !s.contains('@') || s.chars().any(char::is_whitespace))
        {
            return Err(CommandError::bad_request("Invites must be email addresses"));
        }
        if self.membership != WorkChatMembership::Invites {
            self.invites.clear();
        }
        Ok(())
    }
}

/// Existing runtime resource exposed through the Application.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationResourceKind {
    Chat,
    Room,
    Memory,
}
/// One named navigation entry and its explicitly shared resource.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationResource {
    /// Unique navigation path within the Application.
    pub slug: String,
    /// Display label shown to participants.
    pub name: String,
    /// The existing resource type exposed by this navigation entry.
    pub kind: ApplicationResourceKind,
    /// Public ID of a Chat channel, Room, or organization Memory.
    pub resource_id: String,
    /// Allow Memory writes; false for read-only Memory and all other resource kinds.
    #[serde(default)]
    pub writable: bool,
}
/// Author-editable Application presentation, audience and resource bindings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationInput {
    /// Display name, between 1 and 120 characters after trimming.
    pub name: String,
    /// Permanent hostname label, globally unique and reserved after archival.
    pub slug: String,
    /// Overview text for participants, up to 2000 characters.
    #[serde(default)]
    pub description: String,
    /// Audience and optional sidebar configuration.
    #[serde(default)]
    pub config: ApplicationConfig,
    /// Ordered resource bindings; at most 50, with unique paths and IDs.
    #[serde(default)]
    pub resources: Vec<ApplicationResource>,
}
impl ApplicationInput {
    pub fn prepare(&mut self) -> Result<(), CommandError> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty()
            || self.name.chars().count() > 120
            || self.description.chars().count() > 2000
        {
            return Err(CommandError::bad_request(
                "A name of 1–120 characters and description of at most 2000 characters are required",
            ));
        }
        if !valid_slug(&self.slug) || reserved_slug(&self.slug) {
            return Err(CommandError::bad_request(
                "Choose an available name of 3–48 lowercase letters, digits or hyphens, starting with a letter",
            ));
        }
        self.config.prepare()?;
        if self.resources.len() > 50 {
            return Err(CommandError::bad_request(
                "At most 50 resources are supported",
            ));
        }
        let mut slugs = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for r in &mut self.resources {
            r.name = r.name.trim().to_owned();
            if !valid_slug(&r.slug)
                || !slugs.insert(r.slug.clone())
                || !ids.insert(r.resource_id.clone())
                || r.name.is_empty()
                || r.name.chars().count() > 120
                || (r.writable && r.kind != ApplicationResourceKind::Memory)
            {
                return Err(CommandError::bad_request(
                    "Resources need unique names and IDs, valid slugs and explicit memory access",
                ));
            }
        }
        Ok(())
    }
}
pub fn valid_slug(s: &str) -> bool {
    (3..=48).contains(&s.len())
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn reserved_slug(s: &str) -> bool {
    ["login","auth","api","admin","www","app","applications","support","status","mail","id","console","assets"].contains(&s)
        // Random Work Chat hostnames keep their existing namespace.
        || super::super::agent_channels::record::work_chat::host_label_ok(s)
}
/// An organization-owned publication with a permanent name and versioned configuration.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Application {
    /// Stable public Application identifier.
    pub id: ApplicationId,
    /// Display name shown in the editor and published workspace.
    pub name: String,
    /// Permanently reserved hostname label.
    pub slug: String,
    /// Participant-facing overview text.
    pub description: String,
    /// Publication state: draft, published, or archived.
    pub status: String,
    /// Current audience and workspace settings.
    pub config: ApplicationConfig,
    /// Ordered navigation entries and explicit shared-resource grants.
    pub resources: Vec<ApplicationResource>,
    /// Optimistic concurrency version required for updates and status changes.
    pub version: i64,
    /// Published hostname, or null when the deployment has no publication domain.
    pub public_url: Option<String>,
    /// Time the Application and its stable name were created.
    pub created_at: DateTime<Utc>,
    /// Time of the latest configuration or publication change.
    pub updated_at: DateTime<Utc>,
}
pub fn public_url(slug: &str) -> Option<String> {
    let domain = std::env::var("CHAT_DOMAIN").ok()?;
    if domain.is_empty()
        || domain.contains("..")
        || !domain
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
    {
        return None;
    }
    Some(format!("https://{slug}.{domain}"))
}
