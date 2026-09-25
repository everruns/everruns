---
type: Investigation
title: "Slack One-Click Install"
description: "How per-agent Slack apps get created in a customer's own workspace, what a live PoC measured, and why the first design could never have worked."
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
products ask for one click. This concept records how that gap closes, what a
live PoC measured, and the constraint that invalidated the design the PoC was
run to validate.

The headline: **per-agent Slack apps must be created in the *customer's*
workspace, never in ours.** Everything else follows from that. Who holds the
token that creates them is the only real variable, and it decides whether the
customer clicks consent once or once per agent.

That design shipped in #3830 and #602 against a customer-supplied configuration
token. The remaining upgrade — a manager app, which removes the per-agent
consent — is blocked on Slack enrolling us, and is the only open item here.

## The constraint everything hangs on

Three facts, each load-bearing:

1. An app configuration token is scoped to a user **and a workspace**.
   `apps.manifest.create` creates the app *in that workspace*, single-workspace
   by default.
2. The manifest schema carries no public-distribution field. Activating
   distribution is a manual click per app in Slack's own UI.
3. Without distribution, `oauth.v2.access` from a foreign workspace fails.

Together: an app created in the Everruns workspace **cannot be installed into a
customer's workspace** unless a human clicks "Activate Public Distribution" on
it. Per customer. Per agent.

So the original shape — Everruns holds one company-wide config token and mints
an app per customer agent — is not slow or awkward. It does not work.

## What was measured

A live PoC ran against a real workspace with an app configuration token. These
are observed API responses, not readings of documentation.

| Question | Result |
|---|---|
| Does `apps.manifest.create` accept the manifest we generate? | **Yes**, complete — `agent_view`, `assistant:write`, interactivity, all nine bot events |
| Does it return credentials? | **Yes** — `client_id`, `client_secret`, `signing_secret`, `verification_token` |
| Does it verify `request_url` at save time? | **No** — a manifest naming an unreachable host was accepted |
| Can a created app install without a consent screen? | **No** — for an ordinary app, `/oauth/v2/authorize` shows a full consent screen |

Each of those still holds. What did not hold was the inference drawn from them:
the PoC ran entirely inside one workspace, so it never touched the
cross-workspace constraint above, and its conclusions were generalised past
what it measured. See [What this corrected](#what-this-corrected).

## Manager apps

Slack's answer to creating apps in someone else's workspace is the **manager
app**: an app that, after one OAuth consent, may create and configure other
Slack apps in the consenting user's workspace.

Three user scopes, observed on a live competitor authorize URL:

```
scope=                                  (empty — the manager app has no bot of its own)
user_scope=app_configurations:read,app_configurations:write,managed_apps:install
```

Slack's consent UI renders them as "View the configuration of Slack apps on your
behalf", "Create and manage the configuration of Slack apps on your behalf", and
"Install managed child apps on workspaces". The first two are locked; **the
third is a checkbox the user can untick.**

`managed_apps:install` is the one that matters: it lets a manager app install the
child apps it creates, so the customer consents once and never again. It is not
in Slack's published scope reference; the live consent screen is the evidence.

The scopes are gated. From the `apps.manifest.create` reference, error
`invalid_manager_app`: *"The calling app is not enrolled as a manager app.
Ensure the app has `app_configurations:write` in its configured scopes and its
home team has manager app support enabled."* An unenrolled workspace cannot even
declare them — Slack's manifest editor rejects them outright, and they are absent
from the User Token Scopes picker. Enrollment is granted by Slack and is not
self-serve.

Also on that page: `managed_app_limit_reached`. A quota exists on apps created
per manager app. Whether it counts per manager app or per customer workspace is
unknown and materially affects design.

## Decisions

**Create per-agent apps in the customer's workspace.** Not ours. This is the
whole finding, and it is what makes per-agent identity compatible with an
install the customer can actually complete.

**Two token sources, one machinery.** The org's config token either comes from
the customer pasting one they generated in their own workspace, or from a manager
grant once Slack enrolls us. Everything downstream — org-scoped encrypted token
storage, rotation, `apps.manifest.create`, the OAuth callback — is identical.
Building behind that seam is what stopped the enrollment question from blocking.

**The customer-supplied token path ships regardless of enrollment.** It is not a
stopgap. It is the only path before enrollment, the required fallback after it
(a customer may untick `managed_apps:install`, leaving a grant that cannot
install), and the self-hosted story. Vercel, who evidently have enrollment, ship
a "Customer Owned Connector" path beside their managed one for the same reasons.

**The `slack_app_provisioner` hook is an override, not an addition.** A
deployment that supplies its own provisioner gets `connection_manager: None`,
and the connection manager is what backs the per-organization connect, test and
disconnect routes one-click depends on. So filling the hook *disables* one-click
rather than reinforcing it. This is not a subtlety to rediscover: an earlier SaaS
implementation supplied a provisioner precisely to enable one-click and would
have turned it off. It survived only because it was never configured in
production. Deployments leave the hook unset unless they are deliberately
replacing the whole flow.

**Encryption is load-bearing.** With no `EncryptionService`, `configure` installs
no provisioner at all and the deployment falls back to the manual form. A
deployment that wants one-click needs `SECRETS_ENCRYPTION_KEY` set, and its
absence is silent rather than an error.

**A connection can degrade, not just exist or not.** Config token pairs are
single-use and rotate; a rotation that fails leaves the org connected in name
but unable to provision. The capability therefore reports `supported`,
`connected` and `reconnect_required` separately rather than one boolean, and the
form renders the reconnect case as its own state with its own recovery.

**Per-endpoint app identity is kept.** Each endpoint gets its own Slack app,
which is what puts a distinctly named and avatared agent in Slack's Agents menu.
Nothing here trades that away.

**One-click is an OSS capability, not a SaaS feature.** A self-hosted deployment
has a workspace and can generate a config token, so it gets the same path. Only
configuration differs.

## What this corrected

Recorded so they are not re-derived. The first two predate the PoC; the rest are
the PoC's own conclusions, corrected.

- **That one-click and per-agent identity were in tension.** They are not.
  Programmatic creation gives both.
- **That the manifest needs a live endpoint before the Slack app can exist.**
  The web *create from manifest* flow verifies `request_url` on save, which is
  why [Slack Integration Modernization](slack-modernization.md) sequenced
  publish before app creation. The API does not verify it. Whether Slack
  *delivers* events to an unverified URL is a separate question the PoC did not
  answer, so the existing ordering should not be relaxed on this basis alone.
- **That no Marketplace listing is required.** Stated here previously. True only
  within a single workspace, which is all the PoC exercised. Reaching a
  customer's workspace needs either manager enrollment (which the observed
  competitor apps pair with a Marketplace listing) or a token the customer
  supplies. Neither is "nothing".
- **That the config token is an Everruns-the-company secret, and self-hosted
  keeps the copy-paste flow.** Both wrong, and the second followed from the
  first. The token belongs to the workspace the apps are created in — the
  customer's.
- **That one consent per agent is the reachable target.** It is the fallback,
  not the target. `managed_apps:install` makes zero-consent-per-agent reachable,
  conditional on enrollment.

## Known gap, closed in code

The generated manifest declared no `oauth_config.redirect_urls`. Slack rejects
`/oauth/v2/authorize` outright without it — "redirect_uri did not match any
configured URIs" — so no generated app could ever be installed by OAuth. The
omission was invisible because the copy-paste flow never runs OAuth. See
`slack_oauth_redirect_url` in `crates/server/src/api/slack_events/manifest.rs`.

## Open questions

All concern enrollment, and none block the customer-supplied token path.

- What manager app enrollment requires, and whether a Marketplace listing is a
  precondition or a separate track.
- Whether `managed_app_limit_reached` counts per manager app or per customer
  workspace. Per manager app would cap agents across all customers.
- What a manager app can do when the user declines `managed_apps:install`.
- Whether Slack will enable manager app support on a development workspace, which
  would convert a hard block into a parallel track.

Asking Slack is cheaper than further reverse-engineering.

## Files

- `crates/server/src/slack_provisioning.rs` — `configure`, the two early returns above, the provisioner, and the supervised `slack_token_rotation` sweep
- `crates/platform/src/slack_provisioning.rs` — the `SlackAppProvisioner` trait and its unavailable default
- `crates/server/src/storage/org_slack_connections.rs` — per-organization connection storage; migration `145_org_slack_connections.sql`
- `crates/server/src/api/slack_install.rs` — install and connection routes, and their deliberately different auth
- `crates/server/src/api/slack_events/manifest.rs` — manifest generation and the endpoint URLs it declares
- `apps/ui/src/components/apps/channel-form.tsx` — the setup states the capability drives
- [Slack Integration Modernization](slack-modernization.md) — where setup ordering was decided
- [Slack Agent Actions](slack-agent-actions.md) — the capability and approval work this sits beside
