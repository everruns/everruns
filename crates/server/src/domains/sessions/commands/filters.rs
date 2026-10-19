use super::*;

/// Filters shared by `ListSessions` and `GetSessionFacets` so the page and the
/// counts that annotate it can never describe different populations (EVE-852).
#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct SessionFilterArgs {
    /// Exclude the permanent Chat from side-conversation pagination.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub side_chats_only: Option<bool>,
    /// Agent's prefixed public identifier.
    pub agent_id: Option<AgentId>,
    /// Fixed Playground end-user identity.
    pub playground_user_id: Option<everruns_contracts::typed_id::VirtualUserId>,
    /// Return only archived sessions.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub archived_only: Option<bool>,
    /// Case-insensitive title substring match.
    pub search: Option<String>,
    /// Comma-separated sources (`chat`, `api`, `slack`, `ag_ui`, `fcp`,
    /// `schedule`, `webhook`, `a2a`, `eval`, `subagent`, `unknown`).
    pub source: Option<String>,
    /// Comma-separated derived activities (`running`, `paused`, `failed`,
    /// `completed`, `idle`).
    pub status: Option<String>,
    /// Restrict to sessions owned by the calling user.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub mine: Option<bool>,
    /// Inclusive lower bound on `created_at` (RFC 3339).
    pub created_after: Option<String>,
    /// Exclusive upper bound on `created_at` (RFC 3339).
    pub created_before: Option<String>,
    /// Include archived sessions. Default `false`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub include_archived: Option<bool>,
    /// `created_at` (default) or `last_activity`. The chat thread list is this
    /// endpoint with `source=chat`, `mine=true`, `order=last_activity`.
    pub order: Option<String>,
}

impl SessionFilterArgs {
    /// Resolve into the storage-level predicate, translating the agent's public
    /// id and rejecting unknown enum members rather than silently widening the
    /// result set.
    pub(super) async fn resolve(
        self,
        ctx: &Ctx,
    ) -> Result<Option<crate::storage::SessionListFilters>, CommandError> {
        let agent_id = match self.agent_id {
            Some(agent_id) => {
                let row = ctx
                    .db
                    .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
                    .await?;
                // An unknown agent matches nothing; the caller renders an empty
                // page rather than an error.
                match row {
                    Some(row) => Some(AgentId::from_uuid(row.id.uuid())),
                    None => return Ok(None),
                }
            }
            None => None,
        };

        fn parse_csv<T>(
            raw: Option<&str>,
            parse: impl Fn(&str) -> Option<T>,
            field: &str,
        ) -> Result<Vec<T>, CommandError> {
            let Some(raw) = raw else {
                return Ok(vec![]);
            };
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(|part| {
                    parse(part).ok_or_else(|| {
                        CommandError::bad_request(format!("Unknown {field}: {part}"))
                    })
                })
                .collect()
        }

        fn parse_time(
            raw: Option<&str>,
            field: &str,
        ) -> Result<Option<DateTime<Utc>>, CommandError> {
            raw.map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|t| t.with_timezone(&Utc))
                    .map_err(|_| {
                        CommandError::bad_request(format!("{field} must be an RFC 3339 timestamp"))
                    })
            })
            .transpose()
        }

        let order = match self.order.as_deref() {
            None | Some("created_at") => crate::storage::SessionListOrder::CreatedAt,
            Some("last_activity") => crate::storage::SessionListOrder::LastActivity,
            Some(other) => {
                return Err(CommandError::bad_request(format!("Unknown order: {other}")));
            }
        };

        Ok(Some(crate::storage::SessionListFilters {
            include_archived: self.include_archived.unwrap_or(false),
            side_chats_only: self.side_chats_only.unwrap_or(false),
            archived_only: self.archived_only.unwrap_or(false),
            playground_user_id: self.playground_user_id,
            agent_id,
            search: self.search,
            sources: parse_csv(self.source.as_deref(), SessionSource::parse, "source")?,
            activities: parse_csv(self.status.as_deref(), SessionActivity::parse, "status")?,
            // `mine` without an authenticated user would silently list the
            // whole org, so it resolves to "nothing" instead.
            owner_user_id: match self.mine {
                Some(true) => Some(ctx.caller.user_id.unwrap_or(ANONYMOUS_USER_ID)),
                _ => None,
            },
            created_after: parse_time(self.created_after.as_deref(), "created_after")?,
            created_before: parse_time(self.created_before.as_deref(), "created_before")?,
            order,
        }))
    }
}
