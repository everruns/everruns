---
type: Specification
title: "Public Chat (Hosted Chat App)"
description: "Public Chat (hosted, isolated chat app), product spec/proposal."
tags:
  - everruns
  - integrations
---
# Public Chat (Hosted Chat App)

> Status: **Proposal / product spec.** This document captures the product
> intent, user scenarios, and a phased build plan for an isolated, public
> chat experience built on top of Apps and Channels. It deliberately stays
> at product level. Field-by-field shapes, SQL, and API contracts are
> defined when each phase is implemented and linked back from here.

## Abstract

**Public Chat** lets an organization turn one of its Apps into a standalone,
public-facing chat website that end users, people outside Everruns, can
open and talk to. The org owner builds an Agent and a Harness as they do
today, creates an App, and adds a **Public Chat channel**. Publishing the App
deploys an isolated chat UI that anonymous or signed-in visitors can use to
converse with exactly that one App's agent, and nothing else.

The experience is intentionally **isolated** from the main Everruns product:

- A visitor of a Public Chat never sees the Everruns console, other orgs,
  other apps, other agents, sessions, files, or tools.
- The public app authenticates its own visitors (starting with anonymous and
  Google sign-in) using credentials that are unrelated to Everruns platform
  user accounts.
- Each App's chat is isolated from every other App's chat, even within the
  same org.

The first release runs under Everruns' own endpoints (a path under the
existing host). A later release moves each Public Chat onto its own
hostname/custom domain, conceptually like GitHub Pages giving a project its
own URL, without changing the product model. See
[Custom domains (future)](#custom-domains-future).

A second, parallel goal is a **reusable chat UI library** so the same chat
components used inside the Everruns console can also power these public,
eventually white-labeled, chat apps. See
[Reusable chat UI](#reusable-chat-ui).

## Why this fits Everruns today

This concept maps almost entirely onto primitives that already exist. The
spec is mostly about composing them and adding a thin public surface, not
inventing new architecture.

| Need | Existing primitive |
| --- | --- |
| "An App I can expose" | App + publish lifecycle ([apps.md](apps.md)) |
| "Add a channel called chat" | App Channels, one-to-many, typed config ([apps.md](apps.md)) |
| "Talk to exactly one agent" | Channel binds the App's Harness + optional Agent |
| "Public HTTP, streaming replies" | AG-UI public SSE endpoint pattern ([apps.md](apps.md)) |
| "Google sign-in for visitors" | `google_oidc` in the shared endpoint-auth framework ([app-endpoint-auth.md](app-endpoint-auth.md)) |
| "Isolated, no internal leaks" | Public-endpoint error sanitization ([public-endpoints.md](../execution/public-endpoints.md)) |
| "Sessions per visitor" | App-channel session ownership + routing tags ([app-invocation-channels.md](app-invocation-channels.md)) |
| "Org/app isolation" | Organization multitenancy ([multitenancy.md](../security/multitenancy.md)) |
| "Reusable, brandable chat UI" | `apps/ui` chat components + `design-system.css` (already authored as the white-label source of truth) |
| "Own domain later" | Production reverse-proxy contract ([production-deployment.md](../operations/production-deployment.md)) |

The genuinely new work is: (1) a public chat **channel type** with its own
config; (2) an **isolated public chat web app** that renders against that
channel; (3) **end-user identity** for visitors (anonymous + Google) with
optional persisted sessions; and (4) extracting a **shared chat UI package**.

## Feature flag

The entire feature is gated behind the experimental `public_chat` feature flag
(env `FEATURE_PUBLIC_CHAT`; auto-enabled in dev). Gating is layered:

- **Public endpoints** (`/v1/apps/{app_id}/public-chat[/config]`) check the
  deployment-level flag; when off they return a sanitized 404, so no public
  surface exists regardless of channel rows.
- **Channel creation/editing** of a `public_chat` channel checks the
  org-effective flag (system enabled AND org opted in), rejected otherwise.
- **Builder UI** hides `public_chat` from the channel-type picker unless the
  org-effective flag is on.

To take a live chat offline independent of the flag, unpublish the app or
disable the channel (the flag governs availability, not per-channel lifecycle).

## Implementation status

- **Phase 1 (backend), landed.** The `public_chat` channel type and typed
  config are in `crates/platform/src/app.rs` (`PublicChatChannelConfig`,
  `PublicChatBranding`, `PublicChatCaptchaConfig`, `CaptchaProvider`), with
  create/update validation, secret redaction (`token`, Turnstile `secret_key`),
  PATCH secret-preservation, and migration `092_app_channel_type_public_chat.sql`.
  The public ingress endpoint is implemented in
  `crates/server/src/api/public_chat.rs`, reusing the AG-UI streaming core
  (`ag_ui::run_app_agent_stream`, scoped with a `public_chat:` routing-tag
  prefix) and the shared endpoint-auth verifier, with Cloudflare Turnstile
  verification in `crates/server/src/api/turnstile.rs`. Public Chat has its own
  rate-limiter namespace (`public_chat`).
- **Phase 2 (web app), landed (MVP).** An isolated, chrome-free chat surface
  lives in `apps/ui` under the `(public)` route group at
  `/public-chat/{appId}`. It renders against the public endpoints only
  (`apps/ui/src/lib/public-chat/client.ts`, a minimal AG-UI SSE client),
  applies branding, supports anonymous streaming chat with per-browser thread
  continuity, and renders the Cloudflare Turnstile widget
  (`turnstile-widget.tsx`) when a challenge is enforced. It does not use the
  console's auth/org providers or navigation.
  - Known MVP limitation: the server verifies Turnstile on every anonymous
    request, so the widget re-solves after each message (invisible in Turnstile
    managed mode). A future refinement verifies once per session/thread.
- **Phase 4 (builder config UX), landed (MVP).** The console channel form
  (`apps/ui/src/components/apps/channel-form.tsx`) supports a `public_chat`
  channel: anonymous toggle, optional shared token, branding (name, logo,
  primary color, welcome message), Cloudflare Turnstile (site + write-only
  secret key), tool visibility, rate limit, and session expiration. The app
  detail page shows a `PublicChatSetupGuidance` card with the copyable
  shareable link (`/public-chat/{appId}`), and the apps list/channel rows
  surface the link and access summary. The form also has a Google sign-in
  section (see Phase 3).
- **Phase 3 (Google sign-in), landed (MVP).** The public web app renders a
  Google Identity Services sign-in button (`google-signin.tsx`) when the channel
  configures `google_oidc`; the resulting ID token is sent as a bearer token and
  validated by the shared verifier. Sign-in is mandatory whenever a provider is
  configured (the server requires `auth` on every request), and signed-in
  visitors skip the Turnstile challenge. The builder form gained a "Google
  sign-in" section (client ID + optional allowed domains).
- Phases 5–6 not started. See the build plan below.

### Endpoints

All public, app-scoped, and gated on a published App with an enabled
`public_chat` channel. Errors are sanitized and uniform so callers cannot
distinguish "no app" / "unpublished" / "channel disabled".

- `GET /v1/apps/{app_id}/public-chat/config`, non-secret bootstrap for the web
  app: display name, branding (logo, primary color, welcome message), whether
  anonymous access is allowed, the sign-in method (e.g. `google_oidc` + public
  Google client ID), and the Turnstile site key when a challenge is enforced.
  Never returns secrets.
- `POST /v1/apps/{app_id}/public-chat`, AG-UI `RunAgentInput` in, AG-UI SSE
  out. Order of gates: resolve published channel → authenticate (inline `auth`,
  e.g. Google, marks the visitor signed-in; otherwise anonymous + optional
  shared token) → Turnstile challenge for anonymous visitors when enabled →
  per-app/IP rate limit → stream via the shared core. Signed-in visitors bypass
  the Turnstile challenge.

Token headers: `Authorization: Bearer <token>` or
`X-Everruns-Public-Chat-Token` for the optional shared token; the Turnstile
token is read from `cf-turnstile-response` or `X-Everruns-Turnstile-Token`.

## Goals

1. An org owner can, with no code, turn an App into a public chat site by
   adding and configuring a Public Chat channel and publishing the App.
2. A visitor can open the chat URL and converse with that App's agent, with
   streaming responses, on any modern browser including mobile.
3. The visitor experience is strictly scoped to one App, no path to other
   apps, agents, orgs, or any Everruns internals.
4. Support optional **sessions**: a returning, signed-in visitor can resume
   their previous conversation; anonymous visitors get a best-effort
   per-browser session.
5. Support **authentication**, anonymous by default, with **Google sign-in**
   available from the first release.
6. Reuse Everruns' chat UI building blocks via a shared library, with branding
   (colors, name, logo) configurable per channel.
7. Keep the design portable so a later release can serve each chat on its own
   hostname/custom domain.

## Non-goals (first MVP)

1. Custom domains / per-app hostnames. MVP serves under Everruns endpoints.
   The model is designed to make this a later, additive change.
2. Full white-label theming beyond a small branding set (name, logo, primary
   color, welcome message). Deep theming and multi-tenant theme management
   come later.
3. Identity providers beyond anonymous and Google (e.g. GitHub, email magic
   link, generic OIDC), the auth framework already supports more modes, but
   MVP ships and tests anonymous + Google only.
4. Multi-agent or agent-switching within one chat. One Public Chat = one App.
5. Human handoff / live-agent takeover, ticketing, CRM, analytics dashboards.
6. End-user account management surfaces (profile editing, data export self-
   service). Visitor identity is sign-in for continuity only.
7. Payments, end-user billing, or usage metering surfaced to visitors.

## Personas

- **Builder / Org Owner.** Builds the Agent + Harness, creates the App, adds
  and configures the Public Chat channel, publishes it, and shares the URL.
  Works inside the Everruns console.
- **Visitor / End user.** A person outside Everruns who opens the public chat
  URL to get help from the agent. May be anonymous or signed in with Google.
  Never sees the Everruns console.
- **Operator.** Runs Everruns; cares about isolation, abuse/rate limits, cost
  controls, and that public surfaces leak nothing internal.

## User scenarios

### S1, Builder publishes a public chat (no code)

1. Builder has an App bound to a Harness and an Agent.
2. On the App's channels page, Builder adds a channel of type **Public Chat**.
3. Builder configures: display name, optional logo, primary color, a welcome
   message, who can access (anonymous vs. Google sign-in required, optional
   allowed email domains), session behavior, and rate limits.
4. Builder publishes the App.
5. Builder is shown the public chat URL and can copy/share it.

**Acceptance**
- A Public Chat channel can be created, edited, enabled/disabled, and removed
  like any other channel.
- The public URL is reachable only while the App is published **and** the
  channel is enabled; otherwise it returns a sanitized not-available response
  that does not distinguish "no app" / "unpublished" / "channel disabled".
- Editing branding/config takes effect on the public site without redeploying.

### S2, Anonymous visitor chats

1. Visitor opens the chat URL on the channel configured for anonymous access.
2. Visitor sees the branded chat (name, logo, color, welcome message) and a
   message composer.
3. Visitor sends a message and sees a streaming reply.
4. Visitor continues the conversation; context is retained within the session.

**Acceptance**
- First message starts a session owned by the App (App owner principal),
  scoped and tagged to this channel, per the app-channel ownership rules.
- Responses stream token-by-token; a slow/failed turn yields an actionable,
  sanitized message (no provider names, model IDs, stack traces, budget state).
- The visitor cannot reach any other app/agent/org; there is no navigation
  out of the single-App chat.
- Tool activity, if shown at all, is generic/narrated only, never raw tool
  names, arguments, results, or internal IDs (mirrors AG-UI public behavior).

### S3, Visitor signs in with Google and resumes

1. Channel requires (or offers) Google sign-in.
2. Visitor signs in with Google; if the channel restricts allowed domains, an
   out-of-domain account is rejected with a clear message.
3. Visitor's conversation is associated with their verified identity.
4. Visitor returns later, signs in again, and resumes their prior conversation.

**Acceptance**
- Google ID tokens are verified using the shared endpoint-auth verifier
  (`google_oidc`), honoring configured audience and allowed-domain checks.
- A signed-in visitor's sessions persist across visits within the configured
  retention window and resume on re-auth.
- Two different signed-in visitors of the same channel never see each other's
  conversations.
- Auth failures are generic (401/403 class) and do not distinguish
  misconfiguration from credential probing.

### S4, Visitor on mobile / shares a link

1. Visitor opens the URL on a phone; layout is responsive and usable.
2. Visitor refreshes or returns on the same browser; an anonymous session is
   resumed best-effort (e.g. via a per-browser cookie) until it expires.

**Acceptance**
- The chat is responsive and accessible (keyboard, screen-reader labels,
  sufficient contrast) on mobile and desktop.
- Anonymous session continuity works within the configured expiration window;
  an expired/unknown session starts fresh without error.

### S5, Operator protects the surface

1. Operator relies on per-app, per-IP rate limits and global limits.
2. Operator unpublishes or disables the channel to take it offline instantly.

**Acceptance**
- Rate limits are enforced per app/channel and surface `429` with `Retry-After`
  and an actionable body.
- Unpublishing or disabling the channel stops new traffic immediately; in-
  flight/existing sessions are not exposed to new public traffic.
- No public response reveals internal topology, other tenants, or that other
  apps exist.

## Isolation requirements

Isolation is the defining property of this feature and is non-negotiable.

1. **One App only.** A Public Chat surface can reach exactly one App's agent.
   There is no agent/app switching, listing, or discovery from the public app.
2. **No platform-user auth on the public path.** Visitor authentication is
   separate from Everruns user accounts; the console's user-session auth
   middleware is not on the public chat route path. (Same stance FCP takes.)
3. **Tenant isolation.** Visitors of one App's chat cannot read, resume, or
   infer the existence of other apps, agents, orgs, sessions, files, or tools.
4. **End-user isolation.** Distinct visitors (anonymous browser sessions, or
   distinct signed-in identities) never share conversation state.
5. **No internal-state leaks.** All public responses, success and error,
   pass through the public error-sanitization contract
   ([public-endpoints.md](../execution/public-endpoints.md)). Tool surfacing follows the
   AG-UI generic/narrated rules; raw tool internals never reach the wire.
6. **Designed to relocate.** Because the surface is self-contained and does
   not depend on console auth or cross-app data, it can later be served from a
   separate hostname/origin without weakening any of the above.

## Authentication & sessions (product behavior)

Per-channel access policy, anonymous by default:

- **Anonymous**: anyone with the link can chat. Continuity via a per-browser
  session cookie until it expires.
- **Google sign-in**: visitor must authenticate with Google; optional
  allowed-domain restriction. Conversations bind to the verified identity and
  resume on re-auth.

Both modes reuse the shared App endpoint-auth framework
([app-endpoint-auth.md](app-endpoint-auth.md)); Public Chat ships and tests
`anonymous` and `google_oidc` first. Other modes (generic OIDC, OAuth2
introspection, HTTP Basic, mTLS) are available in the framework and can be
exposed later without new architecture.

Session behavior is configurable per channel:

- **Session continuity window**: how long a session can be resumed
  (analogous to AG-UI/FCP `session_expiration_seconds`).
- **Session scope**: one conversation thread per visitor (default). All
  sessions adopt the App's owner principal and channel routing tags so
  budgets, reuse, and audit logging stay consistent with other app channels
  ([app-invocation-channels.md](app-invocation-channels.md)).

## Abuse & cost controls

A public, link-shareable, anonymous-by-default surface needs defense in depth.
The MVP layers cheap-but-effective controls and leans on what already exists,
so the most expensive work (the LLM turn) only runs for traffic that has
cleared the earlier gates.

Layers, in order of when they fire:

1. **Operator kill switch (exists).** Unpublishing the App or disabling the
   channel takes the surface offline immediately. This is the always-available
   escape hatch.
2. **Bot mitigation / CAPTCHA, Cloudflare Turnstile.** Anonymous channels run
   a Turnstile challenge before a visitor can start chatting; the token is
   verified server-side (Turnstile `siteverify`) before any session is created
   or any turn runs. Configurable per channel:
   - **Enabled by default for anonymous access.** Builders can disable it (e.g.
     a trusted embed) but the safe default is on.
   - **Skipped for verified sign-in.** Google-authenticated visitors already
     cleared a stronger identity gate, so they bypass the challenge.
   - Turnstile is the chosen provider for MVP; the config stays
     provider-shaped so another challenge provider could be added later
     without reworking the flow.
3. **Rate limits (exists).** Per-app, per-IP caps (plus the global API limit),
   surfacing `429` with `Retry-After` and an actionable body, same mechanism
   AG-UI/FCP use.
4. **Cost guardrails (exists + thin add).** Public traffic runs under the App's
   owner principal, so existing org/app **budgeting** already bounds spend
   ([budgeting.md](../security/budgeting.md), [usage-tracking.md](../security/usage-tracking.md)). MVP
   relies on that plus rate limits; an optional **per-channel spend/quota cap**
   for public traffic is a fast follow rather than a blocker. When a budget is
   exhausted, the public surface returns a sanitized `service_unavailable`
   (never budget state) per the public-endpoints contract.

All challenge/limit/budget failures stay within the public error-sanitization
contract, generic, actionable, leaking no internal state.

## Reusable chat UI

A core goal is that the chat components powering the Everruns console also
power these public apps, with branding overrides.

Current state (informs, does not constrain, the design):

- The console UI lives in `apps/ui` (Next.js + React + Tailwind).
- Chat components exist under `apps/ui/src/components/chat/`, some are
  presentational and portable (composer, markdown/message rendering, image
  attachments), while orchestration (session/SSE/tool wiring) is tightly
  coupled to console internals.
- `apps/ui/src/app/design-system.css` is already authored as the
  single-source-of-truth design system intended for downstream/white-label
  consumers to import rather than duplicate.
- The SSE transport is already pluggable (an event-stream factory), which
  lets a public app supply its own auth/transport.

Product direction:

1. Extract the portable, presentational chat pieces into a shared package
   (e.g. a `packages/chat-ui` workspace) that both the console and the public
   app consume. Keep console-specific tool cards and session orchestration out
   of the shared layer behind a small interface.
2. The public app provides its own thin data/transport adapter (talking to the
   Public Chat endpoint) and its own minimal session orchestration.
3. Branding (name, logo, primary color, welcome message) is applied by
   overriding design-system CSS variables, no fork of components.
4. Deep white-labeling (full theming, per-tenant themes) is explicitly future.

This extraction can proceed incrementally and partially; the MVP only needs
enough shared surface to avoid forking the message thread and composer.

## Custom domains (future)

MVP serves each Public Chat under an Everruns endpoint (a path on the existing
host). A later phase gives each App its own hostname (a provisioned subdomain
and/or a customer custom domain), conceptually like GitHub Pages.

This is intended to be additive: the public surface is already self-contained
(no console auth, no cross-app data), so relocating it to a dedicated origin is
primarily a routing/TLS/proxy concern handled by the production reverse-proxy
contract ([production-deployment.md](../operations/production-deployment.md)) plus
certificate management and a domain-to-App mapping. Out of scope for MVP
beyond keeping the model relocation-friendly.

## Feasibility assessment

**Verdict: feasible and well-aligned.** The hard, security-sensitive parts,
public ingress, endpoint auth (incl. Google OIDC), error sanitization,
session ownership/isolation, multitenancy, streaming, already exist and are
exercised by AG-UI/FCP/A2A channels. Public Chat is mostly composition plus a
new front-end surface.

- **Lowest risk / mostly reuse:** publish lifecycle, channel CRUD, app-scoped
  public ingress, `google_oidc` auth, public error sanitization, per-app/IP
  rate limiting, session ownership and routing tags.
- **Moderate effort / net-new:** the isolated public chat web app; the Public
  Chat channel type + config (branding, access policy, session settings);
  persisted, resumable per-visitor sessions for signed-in users.
- **Moderate effort / incremental:** extracting the shared chat UI package
  without destabilizing the console chat.
- **Deferred / higher effort:** custom domains and deep white-labeling.

Key risks to manage: (1) strict isolation and leak-free error surfaces on a
brand-new public web app; (2) keeping the shared UI extraction from
regressing the console; (3) abuse/cost control on an anonymous public surface.
All have existing precedents in the codebase to follow (notably FCP's
isolation invariants and AG-UI's public-stream rules).

## Phased build plan

Each phase is independently shippable and PR-sized where possible.

**Phase 0, Spec sign-off & open questions.** Resolve
[open questions](#open-questions); confirm channel-type naming and MVP auth
scope. Deliverable: this spec accepted.

**Phase 1, Public Chat channel (backend).** Add the Public Chat channel type
and config (branding, access policy = anonymous | google_oidc + allowed
domains, session continuity window, rate limits). Wire its public ingress
through the shared endpoint-auth verifier and the public error-sanitization
contract; adopt app-channel session ownership/routing. Reuse AG-UI-style
streaming. Deliverable: a published App with this channel answers public,
streamed chat requests with full isolation; covered by tests mirroring
AG-UI/FCP (auth accept/reject, sanitization, session reuse, rate limit).

**Phase 2, Isolated public chat web app (MVP UI).** A standalone, responsive
chat web app rendering against the Phase 1 endpoint, with branding applied
from channel config, anonymous chat, streaming, best-effort per-browser
session continuity, and the **Cloudflare Turnstile** challenge on anonymous
channels (verified server-side before a session/turn runs). No console chrome,
no cross-app navigation. Served under an Everruns endpoint. Deliverable:
end-to-end anonymous public chat with bot mitigation (S2, S4, S5).

**Phase 3, Google sign-in & resumable sessions.** Add Google sign-in to the
public app, verified via `google_oidc`; bind conversations to the verified
identity; persist and resume signed-in sessions within the retention window;
enforce allowed-domain restrictions. Deliverable: S3 end-to-end.

**Phase 4, Builder configuration UX.** Console UI to add/edit/enable/disable
the Public Chat channel, set branding and access policy, and view/copy the
public URL. Deliverable: S1 end-to-end, no code required.

**Phase 5, Shared chat UI package.** Extract portable chat components into a
shared workspace package consumed by both the console and the public app;
refactor the public app onto it; verify the console chat is unchanged.
Deliverable: one chat UI source feeding both surfaces, branding via CSS vars.

**Phase 6 (future), Custom domains & deeper white-labeling.** Per-App
hostnames/custom domains via the reverse-proxy contract + cert management +
domain↦App mapping; richer theming. Deliverable: chat served on its own
domain. Out of scope for the initial effort.

## Acceptance criteria (rollup)

- A Builder can stand up a working public chat from an existing App without
  writing code, and take it offline instantly by unpublishing/disabling. (S1, S5)
- Anonymous visitors can hold a streaming, context-retaining conversation
  scoped to exactly one App, on mobile and desktop. (S2, S4)
- Google sign-in works with optional domain restriction, and signed-in
  visitors resume prior conversations; visitors never see each other's data. (S3)
- No public response leaks internal state, other tenants, or other apps;
  errors are actionable and sanitized; tool internals are never exposed. (all)
- Rate limits protect the surface and return actionable `429`s. (S5)
- The public chat UI shares its core components with the console chat, themed
  per channel via the design-system variables. (S2, Phase 5)

## Settled decisions

- **Channel type, distinct `public_chat`.** Ships as its own channel type
  (the product is "Public Chat"), internally reusing AG-UI streaming and the
  shared endpoint-auth framework. Keeps the public-web product story and
  isolation invariants clean while maximizing backend reuse.
- **Abuse controls, layered, with Cloudflare Turnstile.** Operator kill
  switch + Turnstile CAPTCHA on anonymous channels (default on, skipped for
  signed-in visitors) + existing per-app/IP rate limits + existing budgeting.
  See [Abuse & cost controls](#abuse--cost-controls).

## Open questions

1. **Anonymous session continuity.** Cookie-based per-browser session only, or
   also a lightweight client-stored token? Affects privacy posture and the
   future custom-domain/cross-origin story.
2. **End-user data lifecycle.** Default retention for visitor conversations;
   whether/how a visitor can clear their own conversation; deletion semantics
   when the channel or App is deleted.
3. **Per-channel spend cap.** Ship the optional per-channel public-traffic
   quota in MVP, or rely on existing org/app budgeting + rate limits first and
   add the cap as a fast follow?
4. **Branding scope for MVP.** Confirm the minimal set (name, logo, primary
   color, welcome message) and defer anything richer to Phase 6.

## Related specs

- [apps.md](apps.md), App + channel model and lifecycle
- [app-endpoint-auth.md](app-endpoint-auth.md), shared inbound auth (Google OIDC, etc.)
- [app-invocation-channels.md](app-invocation-channels.md), session ownership, routing tags
- [public-endpoints.md](../execution/public-endpoints.md), public error-sanitization contract
- [fcp-channel.md](fcp-channel.md), isolation invariants for a public channel
- [multitenancy.md](../security/multitenancy.md), organization isolation
- [budgeting.md](../security/budgeting.md), spend bounds for public traffic
- [usage-tracking.md](../security/usage-tracking.md), LLM token usage tracking
- [authentication.md](../security/authentication.md), platform auth modes (contrast: visitor auth is separate)
- [production-deployment.md](../operations/production-deployment.md), reverse-proxy contract (custom domains, future)
- [brand.md](../ui/brand.md), brand identity, colors, typography
