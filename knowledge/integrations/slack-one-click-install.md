---
type: Investigation
title: "Slack One-Click Install"
description: "What a live PoC established about creating per-agent Slack apps programmatically, and why one consent per agent is the reachable target."
tags:
  - everruns
  - integrations
  - slack
  - saas
---

# Slack One-Click Install

## Abstract

Connecting an agent to Slack asks the operator for four values — signing secret,
bot token, workspace id, channel id — copied out of Slack by hand. Competing
products ask for one click. This concept records what a live PoC against Slack's
App Manifest API established about closing that gap, including the two things it
disproved.

The headline: **three of the four fields can be obtained without a human, using
the manifest we already generate, and no Slack Marketplace listing is required
to do it.** What remains is one consent screen per agent, which is the reachable
target and a large improvement on today.

## What was measured

Run against a real workspace with an app configuration token. Every claim below
is an observed API response, not a reading of the documentation.

| Question | Result |
|---|---|
| Does `apps.manifest.create` accept the manifest we generate? | **Yes**, complete — `agent_view`, `assistant:write`, interactivity, all nine bot events |
| Does it return credentials? | **Yes** — `client_id`, `client_secret`, `signing_secret`, `verification_token` |
| Does it verify `request_url` at save time? | **No** — a manifest naming an unreachable host was accepted |
| Can the created app install without a consent screen? | **No** — `/oauth/v2/authorize` shows a full consent screen |

## Decisions

**Target one consent per agent, not zero.** Programmatic creation plus an
OAuth install replaces all four fields: the signing secret comes back from the
create call, the bot token and workspace id come from the OAuth exchange, and
the channel id is already optional. That is a single button where there are
currently four fields and a trip to api.slack.com.

**A platform-wide marketplace app is not required for this.** It was the
starting assumption and the PoC removed it. Since every app needs its own
consent regardless, a platform app buys nothing for *installation*; it would
only matter if it is what lets a competitor skip the per-agent consent, which is
unproven. Dropping it also drops a Slack Marketplace review from the critical
path — weeks of calendar time, for a step that turns out to be optional.

**Per-channel app identity is kept.** Each channel already gets its own Slack
app, which is what puts a distinctly named and avatared agent in Slack's Agents
menu. Nothing here trades that away, and an approach that collapsed every agent
into one shared bot identity would.

**The config token is a SaaS-side credential.** Creating apps needs an app
configuration token, which is an Everruns-the-company secret rather than
something each self-hosted deployment can hold. Cloud gets the one-click path;
self-hosted keeps the copy-paste flow, unchanged. The OSS manifest generator
serves both — same manifest, different way of getting it to Slack.

## What this corrected

Two things believed before the PoC turned out to be false, both recorded here so
they are not re-derived:

- **That one-click and per-agent identity were in tension.** They are not. The
  fork between "OAuth with one shared bot" and "manifest with per-agent bots"
  was a false choice; programmatic creation gives both.
- **That the manifest needs a live channel before the Slack app can exist.**
  The web *create from manifest* flow verifies `request_url` on save, which is
  why [Slack Integration Modernization](slack-modernization.md) sequenced
  publish before app creation. The API does not verify it. Whether Slack
  *delivers* events to an unverified URL is a separate question this PoC did
  not answer, so the existing ordering should not be relaxed on this basis
  alone.

## Known gap, now closed in code

The generated manifest declared no `oauth_config.redirect_urls`. Slack rejects
`/oauth/v2/authorize` outright without it — "redirect_uri did not match any
configured URIs" — so no generated app could ever be installed by OAuth. The
omission was invisible because the copy-paste flow never runs OAuth. See
`slack_oauth_redirect_url` in
`crates/server/src/api/slack_events/manifest.rs`.

## Open question

How a competitor shows two agent apps in the Agents menu after a single consent
is still unexplained. The live hypothesis is that user-token scopes
("Perform actions as you", requested by their marketplace app) permit
install-on-behalf, but there is no evidence for it beyond the scope request, and
our own config-token path plainly does not get it. Asking them is likely cheaper
than further reverse-engineering. Nothing in the decisions above depends on the
answer.

## Files

- `crates/server/src/api/slack_events/manifest.rs` — manifest generation and the endpoint URLs it declares
- [Slack Integration Modernization](slack-modernization.md) — where setup ordering was decided
- [Slack Agent Actions](slack-agent-actions.md) — the capability and approval work this sits beside
