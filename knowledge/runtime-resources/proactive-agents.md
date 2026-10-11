---
type: Decision
title: "Proactive Agents in Minutes"
description: "Why and how an agent that watches something and tells its owner (a CI watcher) becomes a trigger plus one sentence: CI events on the GitHub trigger, a built-in notify-owner capability, reachable catalog MCP hosts, safe test runs, and a builder that drafts all of it."
tags:
  - everruns
  - runtime-resources
  - triggers
  - notifications
  - agent-builder
---
# Proactive Agents in Minutes

## Abstract

A proactive agent watches something and tells a person when it changes. The
first one we built on app.everruns.com, a "Main CI Watcher" that sends an alert
when `everruns/everruns` main goes red, took a day, three platform fixes and a
dozen manual steps. This concept records the five gaps that build exposed and
the decisions that close them, so the same agent becomes one sentence in the
agent builder: "Tell me when main CI on everruns/everruns breaks or recovers."

## Problem

What building the CI watcher took, in order:

1. **Polling.** A schedule trigger every 15 minutes, fetching
   `actions/workflows/ci.yml/runs` with `web_fetch`. Its 64 KiB cap forced
   `per_page=4`, anonymous GitHub rate limits applied, and because runs are
   stateless the prompt had to infer "just turned red" from the last four runs
   and a 15-minute window. Most runs did nothing and still cost a model call.
2. **Reaching the owner.** There is no way for an agent to tell its owner
   something. Sending one alert needed a hosted MCP service (Visti), a catalog
   preset, an egress allowlist PR (#4428), a personal Connect, and a separate
   agent-level Authorize ([agent MCP attachments](../integrations/agent-mcp-attachments.md)).
3. **Blocked catalog hosts.** Connecting the org's `linear` preset failed with a
   bare 502 because its host was not on the
   [system allowlist](../operations/system-allowlist.md); the GitHub preset's
   `api.githubcopilot.com` still is not. A seeded catalog preset that cannot
   connect on hosted is a trap.
4. **Unsafe test runs.** The first test of `visti_send` ran three times: the
   results were dropped (fixed in #4523), transcript repair told the model the
   call was "cancelled", and the model retried a call that had succeeded. There
   is no way to try an agent without its real side effects.
5. **Fiddly setup.** The schedule was hand-written cron, the system prompt was
   once saved empty by a CLI quirk, and the default model's key had no credit.
   The ["Describe it" builder](../../crates/server/src/domains/agents/draft.rs)
   drafts a prompt, capabilities and channels, but not triggers, MCP servers,
   the model, or a first run.

## Decisions

### D1. CI results are GitHub trigger events

The `github` trigger ([Agent Triggers](agent-triggers.md)) already receives
signed deliveries for the agent's GitHub App and routes them through the shared
event pipeline. CI is two more event types on it, not a new trigger type:

- `workflow_run.completed` and `check_suite.completed` are accepted events.
  The App manifest ([GitHub Apps](../integrations/github-apps.md)) adds
  `actions: read` and `checks: read` and subscribes to `workflow_run` and
  `check_suite`. Existing Apps show "Update permissions on GitHub" on the
  agent's GitHub card; nothing changes for an App until its owner accepts.
- The event context gains a `github.ci` object: workflow name, head branch,
  head SHA, conclusion, run URL, and the run's actor. The subject is
  `owner/repo@branch:workflow`, so `per_thread` keeps one conversation per
  watched workflow and branch.
- **Rejected:** keep polling and fix `web_fetch` limits. Polling stays a model
  call per tick to learn nothing, and GitHub already pushes the event.
- **Rejected:** the generic `webhook` trigger with a repo webhook. It works
  today, but needs repo admin access, a hand-copied secret and payload paths;
  the App already holds a verified, per-repo delivery.

### D2. "Only when it changes" is a trigger filter, not prompt logic

A watcher cares about transitions (green to red, red to green), not every
result. The trigger filter gains `changes_only: { path }`: the pipeline keeps the
last value seen at `path` per trigger and subject, and marks a delivery
`filtered` when the value did not change. The first event for a subject only
records the value. Conclusions are normalized to `success` or `failure` before
comparing (`cancelled` and `skipped` are ignored), so a cancelled run does not
read as a recovery.

The state is one small row per trigger and subject, owned by the
`agent_triggers` domain and deleted with the trigger. It is generic, so a
webhook trigger can use it for any "status changed" payload.

- **Rejected:** shared-session memory, as the watcher uses today. It depends on
  the model reading its own history correctly, and costs a turn per event.

### D3. A built-in "Notify me" capability

A new capability, `owner_notifications` (shown as **Notify me**), gives the
agent one tool, `notify_owner(title, body, url?, urgency?, dedupe_key?)`.

- **Recipients** are people, never addresses: the user who adds the capability,
  plus org members an agent manager picks in its config. The agent cannot name a
  recipient, so it cannot be steered into mailing a stranger.
- **Delivery:** always an in-app notification
  ([Notifications](../operations/notifications.md)), with the agent as `source`
  and kind `agent.notify`. `urgency: high` also sends email through the existing
  [`EmailSender`](../operations/email.md) to recipients who leave email on in
  their notification settings. Web and mobile push come later, as more channels
  behind the same tool.
- **Dedupe and limits:** `dedupe_key` folds repeats into one notification with a
  count (the existing `occurrence_count`). The tool call id is the idempotency
  key, so a retried call never sends twice. A per-agent rate limit (default 20
  per hour) returns a clear tool error.
- **Content:** plain text title and body with a length cap, and an optional
  `https` link. Email renders the agent's name and "sent by an agent", from the
  platform sender.
- No feature flag: the capability is opt-in
  ([feature flag policy](../security/feature-flags.md)).
- This reverses one non-goal in [Email](../operations/email.md), "no agent
  capability or tool for sending email". It stays true for arbitrary
  addresses; the new path only reaches the org's own members, chosen by people.
- **Rejected:** keep using MCP services such as Visti for this. They stay
  available for people who want them, but the default should take zero setup.

### D4. Seeded catalog presets must connect on hosted

- Every seeded catalog preset's host must pass the system allowlist, enforced by
  a test over the seed list. `api.githubcopilot.com` joins `mcp_services`.
- When an admin adds a preset whose host the egress policy would block, the
  reachability check (#4505) reports **Blocked by network policy** instead of
  "unreachable". For an org with the
  [allowlist extension](../operations/system-allowlist.md) granted, it offers
  **Allow this host for the org** (the org admin path from #4452). Otherwise it
  says to ask a platform admin.

### D5. Test runs are safe by default

- **Unknown outcomes are never "cancelled".** When transcript repair
  synthesizes a missing result for a tool that may have run (any `AtMostOnce`
  tool), the text says the outcome is unknown and the call may have completed,
  and tells the model not to repeat it without checking or asking. "Cancelled"
  stays for calls that provably never started.
- **Dry run.** A session can run in dry-run mode. Tools labelled read-only (the
  tool's own annotations or its
  [risk label](../integrations/mcp-servers.md#tool-risk-labels)) run for real. Every other tool
  returns a simulated result naming the call and its arguments, without
  executing. The session trace marks those calls.
- **Test run button.** The agent page gets **Test run**, which starts a
  dry-run session. For a trigger it uses a sample event: the latest real event
  the trigger received, else a canned sample for that event type. A **Run for
  real** toggle turns dry run off.
- **Rejected:** a separate sandbox org for tests. It loses the agent's real
  connections, which are most of what a test should prove.

### D6. The builder drafts the whole agent and runs it once

The "Describe it" builder (`draft.rs`) extends its draft with:

- **Triggers:** a schedule (a cron expression it writes from plain words), or a
  GitHub trigger with events, repositories and `changes_only`. Same filtering
  rule as today's capabilities: only trigger types this org can use.
- **Notify me:** proposed whenever the description says "tell me", "alert",
  "let me know" and similar.
- **MCP servers:** only presets already in the org catalog. The draft never
  adds a host or a credential.
- **Model:** the org default, unless it has no working credential, in which case
  the builder picks the first funded model and says why.

After create, the builder starts a dry **Test run** and shows its trace inline,
so the first thing a person sees is the agent working.

The CLI quirk from the build (a `$(cat <<EOF ...)` inside the MCP `execute`
tool expanding to empty) is a Bashkit bug and is reported there, not worked
around here.

## Success bars

- The Main CI Watcher is rebuilt from one builder sentence: GitHub trigger on
  `workflow_run.completed` for `everruns/everruns` main with `changes_only`,
  Notify me, and no MCP server, schedule or prompt edits by hand.
- Breaking main produces exactly one notification. Fixing it produces exactly
  one "recovered" notification. A run that stays red, or a cancelled run,
  produces none.
- A dry test run of that agent sends nothing and shows the simulated
  `notify_owner` call in the trace.
- No model call happens for a CI event that `changes_only` filters out.
- Every seeded catalog preset connects on a hosted-like egress policy (test).

## Delivery plan

Each step is its own PR, in this order:

1. CI events on the GitHub trigger (D1): manifest permissions and events,
   the `github.ci` context and subject, and the "Update permissions" prompt.
2. `changes_only` filter and its per-subject state (D2).
3. Notify me capability, in-app first (D3).
4. Email delivery for `urgency: high`, with notification settings (D3).
5. Catalog host test, `api.githubcopilot.com`, and the blocked-host message and
   action (D4).
6. Unknown-outcome repair text (D5), then dry-run mode and the Test run button
   (D5).
7. Builder drafts triggers, Notify me, MCP presets and the model, then test-runs
   (D6).
8. Rebuild the Main CI Watcher with the builder as the acceptance test, and
   retire its schedule and Visti attachment.
