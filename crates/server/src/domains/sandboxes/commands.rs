// Org-wide Sandbox fleet commands: list, roll-ups, timeline, one Sandbox.
//
// Decision: reading the fleet uses the Session view policy. A Sandbox is part
// of its Session, and anyone who may open Sessions may see where they ran.
// Lifecycle actions (pause, resume, delete) stay on the Session Sandbox
// endpoint, so the fleet adds no new way to change compute.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::types::*;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use crate::storage::backend::sandbox_fleet::{
    FLEET_STATS_WINDOW_DAYS, LIVE_FLEET_STATES, SandboxFleetFilter, SandboxTransitionRow,
};
use everruns_contracts::typed_id::{AgentId, SandboxId, SandboxTemplateId};

const DEFAULT_PAGE: i64 = 50;
const MAX_PAGE: i64 = 200;
const DEFAULT_LANES: i64 = 50;
const MAX_TIMELINE_DAYS: i64 = 31;

const FLEET_STATES: &[&str] = &[
    "running",
    "paused",
    "lost",
    "starting",
    "failed",
    "not_started",
    "deleted",
];

// The three read commands share one filter vocabulary.
macro_rules! fleet_filter_command {
    ($(#[$meta:meta])* $name:ident { $($(#[$fmeta:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
        #[into_params(parameter_in = Query)]
        pub struct $name {
            /// Comma-separated states: `running`, `paused`, `lost`, `starting`,
            /// `failed`, `not_started`, `deleted`, or `live` for the first five.
            pub state: Option<String>,
            /// Comma-separated provider ids, such as `daytona,modal`.
            pub provider: Option<String>,
            /// Only Sandboxes of Sessions that ran this Agent.
            pub agent_id: Option<String>,
            /// Only Sandboxes created from this Sandbox Template.
            pub sandbox_template_id: Option<String>,
            /// Only Sandboxes with at least one attention reason.
            #[serde(default, deserialize_with = "deserialize_bool_lenient")]
            pub needs_attention: bool,
            /// Case-insensitive match on Session title, Agent, provider or provider resource id.
            pub search: Option<String>,
            /// Include in-process targets (virtual filesystem, host), which have no provider resource.
            #[serde(default, deserialize_with = "deserialize_bool_lenient")]
            pub include_in_process: bool,
            $($(#[$fmeta])* pub $field: $ty,)*
        }

        impl $name {
            fn filter(&self) -> Result<SandboxFleetFilter, CommandError> {
                build_filter(
                    self.state.as_deref(),
                    self.provider.as_deref(),
                    self.agent_id.as_deref(),
                    self.sandbox_template_id.as_deref(),
                    self.needs_attention,
                    self.search.clone(),
                    self.include_in_process,
                )
            }
        }
    };
}

fn split_csv(raw: Option<&str>) -> Option<Vec<String>> {
    let values: Vec<String> = raw?
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
    (!values.is_empty()).then_some(values)
}

fn build_filter(
    state: Option<&str>,
    provider: Option<&str>,
    agent_id: Option<&str>,
    template_id: Option<&str>,
    needs_attention: bool,
    search: Option<String>,
    include_in_process: bool,
) -> Result<SandboxFleetFilter, CommandError> {
    let states = match split_csv(state) {
        None => None,
        Some(values) => {
            let mut states = Vec::new();
            for value in values {
                if value == "live" {
                    states.extend(LIVE_FLEET_STATES.iter().map(|s| s.to_string()));
                } else if FLEET_STATES.contains(&value.as_str()) {
                    states.push(value);
                } else {
                    return Err(CommandError::bad_request(format!(
                        "Unknown Sandbox state '{value}'"
                    )));
                }
            }
            Some(states)
        }
    };
    let agent_id = agent_id
        .map(|raw| {
            raw.parse::<AgentId>()
                .map(|id| id.uuid())
                .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))
        })
        .transpose()?;
    let template_id = template_id
        .map(|raw| {
            raw.parse::<SandboxTemplateId>()
                .map(|id| id.uuid())
                .map_err(|e| CommandError::bad_request(format!("Invalid Sandbox Template ID: {e}")))
        })
        .transpose()?;
    Ok(SandboxFleetFilter {
        states,
        providers: split_csv(provider),
        agent_id,
        template_id,
        needs_attention,
        search,
        include_in_process,
        ids: None,
    })
}

// ============================================================================
// ListSandboxes
// ============================================================================

fleet_filter_command! {
    /// List the organization's Sandboxes, live first.
    ListSandboxes {
        /// Page size, 1 to 200. Defaults to 50.
        limit: Option<i64>,
        /// Rows to skip.
        offset: Option<i64>,
    }
}

#[command(
    name = "list_sandboxes",
    category = "sandboxes",
    description = "List the organization's Sandboxes across providers: running, paused, lost and deleted, with filters.",
    method = "GET",
    path = "/v1/sandboxes",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sandboxes"], "list").with_examples(&[CliExample::new("Find Sandboxes that need attention", "everruns sandboxes list --needs-attention true",)]),
    http = plain,
    params(ListSandboxes),
)]
impl Command for ListSandboxes {
    type Output = SandboxFleetPage;

    async fn execute(self, ctx: &Ctx) -> Result<SandboxFleetPage, CommandError> {
        let filter = self.filter()?;
        let limit = self.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let offset = self.offset.unwrap_or(0).max(0);
        let (rows, total) = ctx
            .db
            .list_sandbox_fleet(ctx.org_id(), &filter, limit, offset)
            .await?;
        Ok(SandboxFleetPage {
            items: rows.into_iter().map(SandboxFleetItem::from).collect(),
            total,
            limit,
            offset,
        })
    }
}

// ============================================================================
// GetSandboxFleetStats
// ============================================================================

fleet_filter_command! {
    /// Roll-ups over the Sandboxes matching the filters.
    GetSandboxFleetStats {}
}

#[command(
    name = "get_sandbox_fleet_stats",
    category = "sandboxes",
    description = "Summarize the organization's Sandboxes: counts by state and provider, recent creations, running time, recoveries and how many need attention.",
    method = "GET",
    path = "/v1/sandboxes/stats",
    policy = SESSION_VIEW,
    http = plain,
    params(GetSandboxFleetStats),
)]
impl Command for GetSandboxFleetStats {
    type Output = SandboxFleetStats;

    async fn execute(self, ctx: &Ctx) -> Result<SandboxFleetStats, CommandError> {
        let filter = self.filter()?;
        let agg = ctx
            .db
            .sandbox_fleet_aggregates(ctx.org_id(), &filter)
            .await?;
        let counts = |pairs: Vec<(String, i64)>| {
            pairs
                .into_iter()
                .map(|(key, count)| SandboxCount { key, count })
                .collect()
        };
        Ok(SandboxFleetStats {
            window_days: FLEET_STATS_WINDOW_DAYS,
            by_state: counts(agg.by_state),
            live_by_provider: counts(agg.live_by_provider),
            created_in_window: agg.created_in_window,
            created_in_prior_window: agg.created_in_prior_window,
            running_seconds_in_window: agg.running_seconds_in_window,
            recoveries_in_window: agg.recoveries_in_window,
            needs_attention: agg.needs_attention,
        })
    }
}

// ============================================================================
// GetSandboxTimeline
// ============================================================================

fleet_filter_command! {
    /// When each Sandbox was running, paused or lost over a window.
    GetSandboxTimeline {
        /// Window start (RFC 3339). Defaults to 24 hours before `to`.
        from: Option<DateTime<Utc>>,
        /// Window end (RFC 3339). Defaults to now.
        to: Option<DateTime<Utc>>,
        /// Lanes to return, 1 to 200. Defaults to 50.
        limit: Option<i64>,
    }
}

#[command(
    name = "get_sandbox_timeline",
    category = "sandboxes",
    description = "Show when each Sandbox was running, paused or lost over a time window, with how many ran at once.",
    method = "GET",
    path = "/v1/sandboxes/timeline",
    policy = SESSION_VIEW,
    http = plain,
    params(GetSandboxTimeline),
)]
impl Command for GetSandboxTimeline {
    type Output = SandboxTimeline;

    async fn execute(self, ctx: &Ctx) -> Result<SandboxTimeline, CommandError> {
        let mut filter = self.filter()?;
        let to = self.to.unwrap_or_else(Utc::now);
        let from = self.from.unwrap_or(to - Duration::hours(24));
        if from >= to {
            return Err(CommandError::bad_request("`from` must be before `to`"));
        }
        if to - from > Duration::days(MAX_TIMELINE_DAYS) {
            return Err(CommandError::bad_request(format!(
                "The timeline window is limited to {MAX_TIMELINE_DAYS} days"
            )));
        }
        let limit = self.limit.unwrap_or(DEFAULT_LANES).clamp(1, MAX_PAGE);

        let rows = ctx
            .db
            .list_sandbox_transitions(ctx.org_id(), None, from, to)
            .await?;
        let spans = spans_by_sandbox(rows);
        if spans.is_empty() {
            return Ok(empty_timeline(from, to));
        }

        // Keep only Sandboxes that also match the fleet filters.
        filter.ids = Some(spans.keys().copied().collect());
        let (items, _) = ctx
            .db
            .list_sandbox_fleet(ctx.org_id(), &filter, spans.len() as i64, 0)
            .await?;
        let mut lanes: Vec<SandboxTimelineLane> = items
            .into_iter()
            .filter_map(|row| {
                let spans = spans.get(&row.id)?.clone();
                Some(SandboxTimelineLane {
                    running_seconds: running_seconds(&spans),
                    sandbox: SandboxFleetItem::from(row),
                    spans,
                })
            })
            .collect();
        lanes.sort_by(|a, b| {
            b.running_seconds
                .cmp(&a.running_seconds)
                .then_with(|| b.sandbox.updated_at.cmp(&a.sandbox.updated_at))
        });

        let (concurrency, peak_running, peak_at) =
            concurrency(from, lanes.iter().flat_map(|lane| lane.spans.iter()));
        let total_lanes = lanes.len() as i64;
        lanes.truncate(limit as usize);
        Ok(SandboxTimeline {
            from,
            to,
            lanes,
            total_lanes,
            concurrency,
            peak_running,
            peak_at,
        })
    }
}

fn empty_timeline(from: DateTime<Utc>, to: DateTime<Utc>) -> SandboxTimeline {
    SandboxTimeline {
        from,
        to,
        lanes: Vec::new(),
        total_lanes: 0,
        concurrency: vec![SandboxConcurrencyPoint {
            at: from,
            running: 0,
        }],
        peak_running: 0,
        peak_at: None,
    }
}

/// Group lifecycle rows into displayable spans. Spans where the Sandbox did not
/// exist at its provider (not started, deleted) are not drawn.
fn spans_by_sandbox(rows: Vec<SandboxTransitionRow>) -> HashMap<uuid::Uuid, Vec<SandboxStateSpan>> {
    let mut grouped: HashMap<uuid::Uuid, Vec<SandboxStateSpan>> = HashMap::new();
    for row in rows {
        let sandbox_id = row.sandbox_id;
        let span = SandboxStateSpan::from(row);
        if span.end <= span.start || matches!(span.state.as_str(), "deleted" | "not_started") {
            continue;
        }
        grouped.entry(sandbox_id).or_default().push(span);
    }
    grouped
}

fn running_seconds(spans: &[SandboxStateSpan]) -> i64 {
    spans
        .iter()
        .filter(|span| span.state == "running")
        .map(|span| (span.end - span.start).num_seconds())
        .sum()
}

/// Step series of running Sandboxes. Ends are applied before starts at the same
/// instant, so a hand-off between two Sandboxes does not count as overlap.
fn concurrency<'a>(
    from: DateTime<Utc>,
    spans: impl Iterator<Item = &'a SandboxStateSpan>,
) -> (Vec<SandboxConcurrencyPoint>, i64, Option<DateTime<Utc>>) {
    let mut deltas: BTreeMap<DateTime<Utc>, (i64, i64)> = BTreeMap::new();
    for span in spans.filter(|span| span.state == "running") {
        deltas.entry(span.start).or_default().1 += 1;
        deltas.entry(span.end).or_default().0 += 1;
    }
    let mut points = vec![SandboxConcurrencyPoint {
        at: from,
        running: 0,
    }];
    let (mut running, mut peak, mut peak_at) = (0_i64, 0_i64, None);
    for (at, (ends, starts)) in deltas {
        running = running - ends + starts;
        if running > peak {
            peak = running;
            peak_at = Some(at);
        }
        match points.last_mut() {
            Some(last) if last.at == at => last.running = running,
            Some(last) if last.running == running => {}
            _ => points.push(SandboxConcurrencyPoint { at, running }),
        }
    }
    (points, peak, peak_at)
}

// ============================================================================
// GetSandbox
// ============================================================================

/// One Sandbox with its provider resources and state history.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetSandbox {
    /// Logical Sandbox id.
    pub id: String,
}

#[command(
    name = "get_sandbox",
    category = "sandboxes",
    description = "Get one Sandbox with its provider resources and full state history.",
    method = "GET",
    path = "/v1/sandboxes/{id}",
    policy = SESSION_VIEW,
    positional = "id",
    cli = CliRoute::new(&["sandboxes"], "get").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("See why a Sandbox was rebuilt", "everruns sandboxes get sandbox_01h9",)]),
    http = plain,
    responses((status = 404, description = "Sandbox not found")),
)]
impl Command for GetSandbox {
    type Output = SandboxFleetDetail;

    async fn execute(self, ctx: &Ctx) -> Result<SandboxFleetDetail, CommandError> {
        let id: SandboxId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid Sandbox ID: {e}")))?;
        let row = ctx
            .db
            .get_sandbox_fleet_row(ctx.org_id(), id.uuid())
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox"))?;
        let incarnations = ctx
            .db
            .list_sandbox_instances(ctx.org_id(), id.uuid())
            .await?;
        let history = ctx
            .db
            .list_sandbox_transitions(ctx.org_id(), Some(id.uuid()), row.created_at, Utc::now())
            .await?;
        Ok(SandboxFleetDetail {
            sandbox: SandboxFleetItem::from(row),
            incarnations: incarnations.into_iter().map(Into::into).collect(),
            history: history.into_iter().map(Into::into).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(state: &str, start_min: i64, end_min: i64) -> SandboxStateSpan {
        let base = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        SandboxStateSpan {
            state: state.to_string(),
            generation: 1,
            start: base + Duration::minutes(start_min),
            end: base + Duration::minutes(end_min),
            current: false,
        }
    }

    #[test]
    fn live_expands_and_unknown_states_are_rejected() {
        let filter = build_filter(Some("live,deleted"), None, None, None, false, None, false)
            .expect("valid states");
        let states = filter.states.expect("states");
        assert!(states.contains(&"running".to_string()));
        assert!(states.contains(&"failed".to_string()));
        assert!(states.contains(&"deleted".to_string()));
        assert!(!states.contains(&"not_started".to_string()));

        let err = build_filter(Some("runing"), None, None, None, false, None, false)
            .expect_err("typo must not silently match nothing");
        assert!(err.to_string().contains("runing"));
    }

    #[test]
    fn blank_filters_mean_no_filter() {
        let filter = build_filter(Some(" , "), Some(""), None, None, false, None, false).unwrap();
        assert!(filter.states.is_none());
        assert!(filter.providers.is_none());
    }

    #[test]
    fn concurrency_counts_overlap_and_ignores_paused_time() {
        let spans = [
            span("running", 0, 30),
            span("running", 10, 20),
            span("paused", 0, 60),
            span("running", 30, 40),
        ];
        let from = spans[0].start;
        let (points, peak, peak_at) = concurrency(from, spans.iter());
        assert_eq!(peak, 2);
        assert_eq!(peak_at, Some(spans[1].start));
        let series: Vec<i64> = points.iter().map(|p| p.running).collect();
        // 0..10 one, 10..20 two, 20..40 one (hand-off at 30 is not overlap), then zero.
        assert_eq!(series, vec![1, 2, 1, 0]);
    }

    #[test]
    fn running_seconds_sums_only_running_spans() {
        let spans = vec![
            span("running", 0, 10),
            span("paused", 10, 50),
            span("running", 50, 55),
        ];
        assert_eq!(running_seconds(&spans), 15 * 60);
    }
}
