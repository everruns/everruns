---
type: Specification
title: "Slack Response Policy"
description: "Decide whether a Slack agent should participate before starting a turn."
tags:
  - integrations
  - slack
---
# Slack Response Policy

An ambient Slack message is context, not necessarily a request. A specialized
agent should not acknowledge personal notes or conversations between other people.
Participation is independent of session binding and outbound reply delivery.

## Availability

Response policies are available on every Slack endpoint without deployment or
organization enrollment. Existing endpoints retain their all-message default;
builders choose filtering explicitly. Selecting all messages restores the previous
behavior. The policy contract lives in
[`slack_channel.rs`](../../crates/platform/src/slack_channel.rs).

## Participation

Mentions and direct messages bypass semantic classification: the user explicitly
invoked the agent. For unmentioned messages, a mentions-only endpoint stays silent;
a relevance-filtered endpoint asks the deployment's existing
[DecisionsService](../operations/decisions-service.md) one yes/no question.

The judgment is whether the message warrants this agent's participation, not just
whether it mentions the agent's topic. A clear in-scope request or a contextual
follow-up qualifies. Personal notes, conversation between other people, mere topic
overlap, and uncertain intent do not. Code owns the response threshold; the initial
value is an experiment requiring domain evaluation, not an accuracy guarantee.
Label-only drivers are consumed as labels, never reported as calibrated certainty.

The state includes bounded agent purpose, current message, attachment names, and
the most recent persisted messages belonging to this Slack thread. Shared sessions
must not contribute messages from other threads or channels. Assistant context is
correlated to matching inputs, and the history is explicitly partial: ignored or
unseen Slack messages are not persisted as agent input. The purpose follows the
version selected for the existing session or the version a new session would use;
classification and session creation share the version-selection helper.

The gate runs in background message processing, after Slack's transport ACK and
before session creation, user resolution, agent execution, progress acknowledgement,
or delivery registration. Missing service, bad verdict, storage failure, and timeout
mean silence for unmentioned messages. Explicit invocations remain available.

[`response_policy.rs`](../../crates/server/src/api/slack_events/response_policy.rs)
owns the question, input bounds, threshold, deadline, and thread isolation. Logs
record the endpoint, verdict, probability, calibration, and answering model without
recording message text. The classifier's vendor receives the bounded state through
the deployment-owned decisions service; agent-facing Jev credentials are not used.

## Validation

Deterministic tests cover threshold boundaries, unavailable and invalid verdicts,
deadline expiry, explicit invocations, legacy defaults, thread isolation, bounded
Unicode input, pinned purpose, and skipped messages leaving no session behind.
Endpoint command tests cover selecting and resetting policies without feature
enrollment. UI tests cover availability, selection, and round-trip preservation.

A separately ignored live Jev test checks representative requests, personal notes,
topic-only statements, and a contextual follow-up. It requires the deployment's
utility key and must be run before claiming domain accuracy.
