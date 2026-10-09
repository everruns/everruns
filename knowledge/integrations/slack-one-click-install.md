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
| Does `apps.manifest.create` accept the manifest we generate? | **Yes**, after encoding its content as a JSON string — `agent_view`, `assistant:write`, interactivity, all nine bot events |
| Does it return credentials? | **Yes** — `client_id`, `client_secret`, `signing_secret`, `verification_token` |
| Does it verify `request_url` at save time? | **No** — a manifest naming an unreachable host was accepted |
| Can a created app install without a consent screen? | **No** — for an ordinary app, `/oauth/v2/authorize` shows a full consent screen |

The manifest's content is accepted, but the API's wire encoding matters: the
copy-paste UI accepts YAML, while the manifest API requires a JSON-encoded
string. A read-only validation probe against the connected production workspace
on 2026-10-03 rejected YAML as `invalid_manifest` and accepted the equivalent
JSON. `SlackApiProvisioner::create_app` now converts the generated YAML at the
API boundary; its regression test checks the actual request body.

What did not hold was the inference drawn from the original PoC:
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

`managed_apps:install` is the one that matters. Its scope reference says it
*"lets a manager app request the installation of managed apps for a user
automatically, without that user going through the OAuth flow for each one."*
The customer consents once and never again.

It is **reserved for Slack partners**: *"This scope is reserved for Slack
partners that use the Add to Slack feature to create Slack apps on behalf of
their users."* That is a business relationship, not a setting support can flip.
It is consistent with the observed competitor apps both carrying the "App is
approved by Slack" badge. From our side the gate shows up as `invalid_manager_app`
on `apps.manifest.create` (*"its home team has manager app support enabled"*),
as Slack's manifest editor rejecting the scopes outright, and as their absence
from the User Token Scopes picker.

Two further properties from the same reference, both relevant once the gate
lifts:

- **Admin approval is not bypassed.** *"Install attempts do not bypass your admin
  approved apps settings."* In a workspace with app approval on, every child app
  needs admin sign-off. `admin.apps.approve` can create a rule that pre-approves
  future child installs from a given manager app; nothing here builds that yet.
- **The manager app itself still needs public distribution**, since it is
  installed into customers' workspaces. That part is self-serve — complete the
  checklist and click Activate Public Distribution. Slack calls the result an
  *unlisted distributed app*. A Marketplace listing is a separate, optional step
  and is **not** required to distribute.

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

**One Everruns agent is one Slack agent. This is a product requirement, not a
preference.** Each endpoint gets its own Slack app, which is what puts a
distinctly named and avatared agent in Slack's Agents menu.

Managed apps follow the agent's saved display name (falling back to its name)
and description after edits, upserts, and version rollbacks. Identity updates
preserve the exported Slack configuration, including permissions, endpoint URLs,
agent-surface settings and suggested prompts. They run off the mutation request
path; failures leave the saved agent intact and are logged. Manual apps remain
operator-managed. See [identity propagation](../../crates/server/src/domains/agents/branding_slack.rs)
and [manifest patching](../../crates/server/src/channels/slack/provisioning/branding.rs).

That rules out the cheapest true one-click: a single publicly distributed
Everruns app behind a standard "Add to Slack" button. It is self-serve and needs
neither a Marketplace listing nor partner status, but one app is one bot, so
every agent would speak as the same identity and share one Agents-menu entry.
Rejected for that reason alone. Do not reopen it as a shortcut to one-click;
the trade it makes is the one this product exists not to make.

The consequence is that per-agent one-click goes through the partner-reserved
manager app or not at all, and the customer-supplied token path is the design
until then.

**A workspace is connected once, by an admin, as organization setup.** An
organization connects any number of Slack workspaces in settings; each
connection records which workspace it is (Slack's rotate response carries
`team_id`), so the UI can name it and the consent screen can be pre-selected
with `team=`. Builders putting an agent in Slack only choose among connected
workspaces and never handle a token. With one workspace there is nothing to
choose; with several the server refuses to guess, so an agent cannot land in
the wrong workspace because a second one was connected later.

**An endpoint is pinned to the workspace its app was created in.** An app lives
in exactly one workspace, so retries, consent and reaping all target that one.
A changed choice before install reaps the unused app and creates a fresh one; an
app already installed is never reaped to satisfy a changed choice.

**Disconnecting a workspace does not take agents offline.** Each agent app holds
its own bot token, independent of the configuration token. Disconnecting removes
only the ability to create agent apps there or update existing ones, and the UI
says so before it happens.

**Removing an Agent removes the managed Slack identity.** Channel deletion,
Agent archive, and permanent deletion remove apps created by Everruns; archive
preserves the Agent definition but requires Slack reinstallation if restored.
Managed removal requires the existing Agent integration-deletion permission,
including through archive. Manual apps stay operator-managed and confirmations
distinguish that limit.
Cleanup must finish before lifecycle success is reported. Because external
deletion cannot roll back, each successful app removal clears its local
installation credentials durably even if another app fails. Retrying skips
completed work. Agent-scoped installation locks prevent concurrent consent
flows from recreating removed apps. Settings and delivery-evidence writes share the
channel lock and re-read the current installation so stale writes cannot restore
removed credentials. Archived Agents cannot start or finish
an installation. See [cleanup](../../crates/server/src/domains/agent_channels/slack_cleanup.rs)
and [regression coverage](../../crates/server/src/domains/agents/lifecycle_slack_tests.rs).

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
- **That nothing stands between us and one-click.** Stated here previously as
  "no Marketplace listing is required". The listing part turned out to be true —
  distribution is self-serve — but the conclusion drawn from it was not. The PoC
  ran in a single workspace and never met the real gate: per-agent apps in a
  customer's workspace need either partner status (for the manager app) or a
  token the customer supplies. Neither is nothing.
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
`slack_oauth_redirect_url` in `crates/server/src/channels/slack/events/manifest.rs`.

Consent also has to request the bot permissions explicitly. Declaring them in
the manifest does not replace requesting them in the OAuth URL. The manifest
and install URL now use the same scope source, including the additive agent
surface permission, so the installed bot receives what the endpoint needs.

Setup must also work before publication. The authenticated install action and
nonce-protected callback resolve a draft endpoint without applying the webhook's
liveness check. Incoming Slack events remain blocked until the endpoint is live;
installation alone never publishes it. The install integration test exercises
consent, token persistence, invalid state, replay rejection, and blocked draft ingress.

The OAuth callback returns to the owning agent's endpoint editor after a valid
install nonce, including declined consent and exchange failures. Unverified,
replayed, or unknown callbacks return to the agents list without revealing the
owning agent. Both destinations are real UI routes; installation must not leave
the operator on a 404 after credentials have been saved.

Native channel config writes must not decode the archival App-linked row: native channels have no `app_id`. The ingress config writer updates the encrypted transport payload directly, preserves first-class channel authentication, and requires an existing row. The installation regression exercises both memory and PostgreSQL storage.

See the [installation regression](../../crates/server/tests/domain/slack_install_integration_test.rs).

## Open questions

All concern partner status, and none block the customer-supplied token path.
Whether a managed app needs its own consent, and whether a Marketplace listing is
needed to distribute, were open here previously; both are answered above from
Slack's documentation.

- What Slack partner status requires, and its timeline.
- Whether `managed_app_limit_reached` counts per manager app or per customer
  workspace. Per manager app would cap agents across all customers.
- What a manager app can do when the user unticks `managed_apps:install`.
- Whether Slack will enable manager app support on a development workspace while
  a partner application is assessed, which would let the build proceed in
  parallel.

These are questions for a Slack partnership conversation, not for further
reverse-engineering.

## Files

- `crates/server/src/channels/slack/provisioning/mod.rs` — `configure`, the two early returns above, the provisioner, and the supervised `slack_token_rotation` sweep
- `crates/server/src/records/slack_provisioning.rs` — the `SlackAppProvisioner` trait and its unavailable default
- `crates/server/src/storage/org_slack_connections.rs` — per-workspace connection storage; migrations `145`, `146` and `159`
- `crates/server/src/channels/slack/install.rs` — install and connection routes, and their deliberately different auth
- `crates/server/src/channels/slack/events/manifest.rs` — manifest generation and the endpoint URLs it declares
- `apps/ui/src/components/apps/channel-form.tsx` — the setup states the capability drives, and the workspace choice
- `apps/ui/src/components/slack/slack-workspaces.tsx` — the connect wizard and token-shape check; the settings page lives at `apps/ui/src/app/(main)/settings/slack/`
- [Slack Integration Modernization](slack-modernization.md) — where setup ordering was decided
- [Slack Agent Actions](slack-agent-actions.md) — the capability and approval work this sits beside
