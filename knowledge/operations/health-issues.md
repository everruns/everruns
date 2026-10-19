---
type: Specification
title: Actionable Health Issues
description: Persistent operational issues surfaced through existing notifications with permission-aware fixes and verified recovery.
tags:
  - everruns
  - operations
  - notifications
  - integrations
---

# Actionable Health Issues

## Intent and boundaries

Give operators one place to discover what needs attention, understand the impact,
and take a recovery action. Operational health is deterministic integration state;
[Agent Checks](../evaluation/agent-checks.md) remain advisory and explicitly requested.
Background reconciliation never runs LLM smoke tests.

A notification records an announcement. A health issue records a condition still
requiring attention. Canonical issue state owns identity and recovery; existing
[notifications](notifications.md) own per-user viewed state, toasts and SSE delivery.
Reading or snoozing an announcement cannot resolve the underlying condition.

The first detector covers Slack installation permissions and rejected bot credentials.
Other integrations can extend this domain without creating another inbox.

The second detector is organization-level: the active-turn limit
(`ORG_MAX_ACTIVE_TURNS`, see [load testing](load-testing.md)). A message refused at
the limit opens an issue with no agent or channel, so members learn why their
messages fail; operators get the same signal as a warning log (Sentry in SaaS) and
the `everruns_org_active_turn_cap_rejections_total` counter. Recording is best effort
and outside the refused request. The issue resolves once the organization runs fewer
turns than the limit: a one-minute sweep and **Check again** recount, since sessions
leave the active state in many places. The limit is protective and approximate,
so the issue may open slightly above or below it. Custom
detectors, automatic repairs, background LLM analysis and external delivery are outside
this delivery's scope.

## User experience

The existing bell has **Action required** above **Activity**. Its number remains the
unread announcement count; unresolved health has a separate labeled count and warning
indicator. Active issues stay visible after an announcement is read. The complete,
paginated list lives in **Settings → Health**, with contextual warnings beside Slack
channel settings.

An issue explains the affected agent, impact, missing permissions, evidence freshness
and recovery action. **Check again** verifies current state; **Remind me tomorrow**
snoozes only the current user's reminder. Snoozing does not hide the issue. Technical
error codes are collapsed; credentials and raw provider responses are never displayed.

Missing `reactions:write` says the bot cannot add emoji reactions. It does not claim
message replies are healthy or declare the whole agent unavailable. Other missing
permissions use a broader partial-functionality warning.

Managed installations reconnect through the existing Slack consent flow. The existing
app manifest is exported and its required scopes are extended while preserving other
configuration. Customer-managed installations get instructions to add the scopes,
reinstall the app and save refreshed credentials through the existing channel editor.
No replacement bot is created to repair an installed app. Slack may require a workspace
administrator to approve the change.

See [Slack One-Click Install](../integrations/slack-one-click-install.md) and Slack's
[scope installation guidance](https://docs.slack.dev/app-management/quickstart-app-settings/).

## Lifecycle and evidence

One condition is deduplicated by organization, channel and detector. Repeated
failures keep the same episode and preserve acknowledgments. Reappearance after
verified recovery opens a new episode with a fresh announcement.

Only a current observation for the same channel revision can resolve an issue.
Timeouts, unavailable scope headers and workspace mismatches retain unknown or stale
health. A previously confirmed failure retains its missing permissions when a probe
becomes unavailable. Disabling the integration or removing credentials makes the
condition inapplicable; deletion removes its linked announcements. Archived agents
and disabled channels are excluded from active projections.

Scope verification uses Slack's non-mutating `auth.test` scope header and workspace
identity. OAuth success alone cannot prove recovery. The callback checks the returned
app and workspace and consumes its nonce even when consent or exchange fails. It
then stores credentials and performs a fresh permission check. Permission checks
never react to a customer message as a probe.

The [domain](../../crates/server/src/domains/health_issues/service.rs) reconciles existing
Slack channels on startup and in bounded, paced sweeps. Configuration changes and
successful OAuth callbacks trigger checks. Typed runtime reaction-permission failures
record the condition immediately; formatted errors or model text cannot create issues.
Missing scopes are permanent action errors rather than retryable provider failures.
Manual checks have a cooldown serialized with installation replacement across instances.

## Announcements and access

The issue is also durable delivery intent. On an eligible user's next health refresh,
the existing notification store idempotently projects one announcement per episode.
This supports users who gain access after detection and replays safely after restart.
It does not broadcast to every organization member. A due snoozed reminder reopens the
same announcement once; resolution updates the linked announcement and its viewed
state. Health lists and counts are independent of the recent activity feed's limits.

Every list, detail, count and action enforces organization scope and the deployment's
active agent policy resolver. Membership is rechecked at delivery, including on an
already-open notification stream. Stale notification rows cannot preserve revoked
access. Repair additionally requires agent management. Recovery uses the existing
settings and OAuth handlers; no provider-supplied command or arbitrary repair URL is
executed. New Slack permissions are granted only through provider consent.

## Success bar and implementation

The regression suite covers repeated failures after reading, recurrence, independent
snooze state, due reminders, replacement credentials, unknown
probe evidence, concurrent checks, revoked membership and cross-organization access.
PostgreSQL-backed API tests exercise persistence, fresh resolution and channel
delete behavior. Slack fixtures verify cancelled/replayed callbacks, wrong app/workspace
rejection and manifest preservation.

Source owns concrete shapes and commands:

- [API](../../crates/server/src/api/health_issues.rs),
  [commands](../../crates/server/src/domains/health_issues/commands.rs),
  [storage](../../crates/server/src/storage/repositories/health_issues.rs).
- [Bell](../../apps/ui/src/components/layout/notification-bell.tsx),
  Health page: `apps/ui/src/app/(main)/settings/health/page.tsx`,
  [issue details](../../apps/ui/src/components/health/health-issue-details.tsx).
- [Slack manifest](../../crates/server/src/channels/slack/events/manifest.rs),
  [install flow](../../crates/server/src/channels/slack/install.rs),
  [provisioning](../../crates/server/src/channels/slack/provisioning/mod.rs),
  [typed runtime errors](../../crates/server/src/channels/slack/actions/mod.rs).
- [Domain tests](../../crates/server/src/domains/health_issues/tests.rs),
  [API tests](../../crates/server/tests/domain/health_issues_test.rs),
  [OAuth tests](../../crates/server/tests/domain/slack_install_integration_test.rs).
