---
type: Specification
title: "Sign-up Experience Redesign — Analysis & Design Brief"
description: "Design brief for on-brand sign-up / onboarding screens."
tags:
  - everruns
  - ui
---
# Sign-up Experience Redesign — Analysis & Design Brief

Status: design brief (not yet a spec). Purpose: give a design agent enough context to
produce on-brand sign-up / onboarding screens for Everruns SaaS. Engineering will
implement from the resulting designs.

Implementation status: the resulting "Onboarding Arc" design (Frames 1–6) is
implemented — persistent shell (`OnboardingShell`/`AuthShell` + stepper + navy
brand panel), unified `/login` door (see `knowledge/security/authentication.md` § Unified
Entry), verify-email under the shell, org creation (`ZeroOrgOnboarding`), the
setup wizard as Configure, and the Done/first-action screen. Wrappers extend
the stepper via `OnboardingArcProvider` (`onboarding-arc-context.tsx`).

---

## 1. What exists today

### 1a. SaaS sign-up (the part that looks bad)

The current SaaS flow, screen by screen:

1. **Auth (PropelAuth hosted pages).** `/login` and `/register` are thin client-side
   redirects to PropelAuth's hosted login/signup. We do **not** own these screens —
   they're PropelAuth's default forms on a PropelAuth domain. Our only styling is the
   `bg-brand-dots` page behind the redirect flash.
   - Files: `apps/saas-ui/src/app/(auth)/login/page.tsx`, `(auth)/register/page.tsx`
2. **Email verification gate.** New users land on `/onboarding`. If their email isn't
   verified, they see a small centered card: "Verify your email", with Refresh / Resend /
   Use-another-account buttons. (Anti-abuse backstop, enforced server-side too.)
3. **Org name prompt.** Once verified, the same small centered card asks for one thing —
   "Organisation name" — with a "Create organisation" button.
   - File: `apps/saas-ui/src/app/(onboarding)/onboarding/page.tsx`
4. **Hand-off to OSS setup.** On submit, the org is created and the user is pushed to
   `/orgs/[orgId]/setup`, which is the OSS-owned setup wizard (re-exported, not overridden).
5. **Dashboard.**

The thing the user finds ugly: steps 2–3 are a **single 512px card floating in the dead
center of an empty dotted page**. It's functional but feels like an afterthought — no
narrative, no brand presence, no sense of progress, no indication of what comes next.

### 1b. OSS setup wizard (the part that's "quite nice")

After org creation, `/orgs/[orgId]/setup` runs a genuinely nice flow we should treat as the
quality bar and the visual anchor:

- **Animated provisioning checklist** — 4 steps reveal/complete ~400ms apart:
  1. Organization created
  2. Harnesses initialised
  3. Default settings configured
  4. LLM provider configured

  Each row animates pending (hollow) → in-progress (spinner) → complete (green check).
- **Inline LLM provider picker** — a 2-column card grid (OpenAI / Anthropic) with provider
  icons and a radio selection state, then a password API-key field with "stored securely
  and encrypted" helper text, plus a "Skip for now" escape hatch.
- File: `apps/ui/src/app/(main)/orgs/[orgId]/setup/page.tsx`

This wizard already embodies the brand: it has rhythm, real-time feedback, and inline
configuration so the user never leaves the flow. **The redesign goal is to make the whole
sign-up arc feel like this — not just the last leg.**

### 1c. OSS auth (for reference / parity)

OSS has its own fully-designed login/register cards (logo, "Welcome back", OAuth buttons,
email+password, on-brand). SaaS doesn't use them because SaaS auth is delegated to
PropelAuth. Worth noting as a reference for tone/copy even though SaaS can't reuse the forms
directly.

---

## 2. The design system (Slate) — non-negotiables for the design agent

Brand personality: **precision instrumentation** — sober, dense, engineered, calm. "We handle
chaos so you don't have to." Logo: three interlocking Borromean rings (durability ×
scalability × reliability).

- **Sharp corners everywhere — 0px radius.** This is a defining trait. No rounded cards,
  buttons, or inputs. (Only exception in the whole app is one experimental badge.)
- **Flat. No drop shadows.** Hierarchy comes from tone + 1px borders, never elevation.
- **Palette:** Navy `#0A1636` (primary), single Gold accent `#D4A43A` (focus rings, active
  states, CTAs), near-white background `hsl(0 0% 98%)`, pure-white cards, gray borders
  `hsl(0 0% 88%)`. Gold is reserved — used sparingly for emphasis. Never gold text on gold.
- **Typography:** Geist Sans (UI) + Geist Mono (code/logs). Tight tracking (-0.01em global,
  -0.02em on headlines). Headlines 600 weight.
- **Texture:** `bg-brand-dots` — a 24px navy dot grid at 8% opacity (gold in dark mode).
- **Spacing:** 4px base scale. Form rows `space-y-4`; label→input `space-y-2`; card padding
  `p-8`.
- **Buttons:** navy default, gold `accent` variant for hero CTAs; full-width in single-column
  forms; icon + label (e.g. trailing `ArrowRight`); gold 3px focus ring.
- **Motion vocabulary already in the system:** 220–260ms ease-out enter animations, a subtle
  1.8s "progress sheen", step-reveal cadence ~400ms. Lean on these — don't invent a new
  motion language.

Source of truth: `apps/ui/src/app/design-system.css` + `apps/ui/DESIGN.md` + `knowledge/ui/brand.md`.

---

## 3. Problems to solve

1. **No brand presence.** Sign-up is the first impression and currently it's a naked form on
   an empty page. There's no logo, no tagline, no product personality, no "why".
2. **No sense of journey / progress.** The user can't see they're 2 steps into a 4-step arc.
   Each screen feels isolated.
3. **Visual discontinuity.** The ugly centered card (steps 2–3) collides jarringly with the
   polished animated setup wizard (step 4+). It should feel like one continuous experience.
4. **Org-name step is thin.** Asking for one field on its own screen wastes a moment that
   could carry brand or set expectations. (We can't merge it into PropelAuth, but we can make
   it feel intentional.)
5. **PropelAuth handoff is abrupt.** The user bounces to an external-looking domain and back.
   We can't redesign PropelAuth's form, but we can frame the entry and return so it feels
   contained. (PropelAuth hosted-page theming is configurable — flag for the design agent to
   consider, but treat the form itself as out of our direct control.)

---

## 4. Proposed ideal user flow (the narrative to design for)

Design the whole arc as **one coherent, progress-aware journey** with a persistent brand frame.
Proposed steps:

**Step 0 — Landing / entry (optional but recommended).**
A branded "Get started" surface that sets context before bouncing to PropelAuth: logo,
one-line value prop ("Durable agentic harness. Unstoppable agents."), and a single primary
CTA. This makes the PropelAuth jump feel like a deliberate step rather than a redirect.

**Step 1 — Authenticate (PropelAuth).**
Out of our direct control, but: (a) consider theming PropelAuth hosted pages to match Slate
(navy/gold, sharp corners, logo); (b) frame the return so the user lands back inside our
branded shell smoothly.

**Step 2 — Verify email (when needed).**
Keep, but reframe as a clearly-labelled stop in the journey, not a dead-end card. Show it as
the current step in the persistent progress frame. Friendly, reassuring copy.

**Step 3 — Create organisation.**
The single org-name field, but presented inside the branded frame with a sense of "this is
your workspace". Could pair the field with a short explainer of what an org is (owns your
agents, sessions, settings) and a reassurance ("invite your team later").

**Step 4 — Provision + configure (existing OSS wizard).**
The animated 4-step checklist + inline provider/API-key setup. This is the payoff. The
redesign should make steps 0–3 visually lead *into* this, so it feels like the climax of one
flow rather than a separate app.

**Step 5 — Land in dashboard / first action.**
Ideally drop the user somewhere actionable (e.g. straight into creating their first agent),
not a cold empty dashboard.

### The unifying idea: a persistent onboarding shell

The strongest single recommendation: wrap steps 2–4 in **one persistent layout** that carries:
- the Everruns logo / brand mark (top-left or centered header),
- a **progress indicator** (e.g. "Verify → Create org → Configure → Done" with the current
  step highlighted in gold),
- consistent card sizing, spacing, and the dotted background,
- optionally a calm right-hand or background panel reinforcing brand (tagline, the Borromean
  rings motif, or a quiet illustration) so the form isn't alone in the void.

This turns "form floating in space" into "guided setup", and makes the seam between SaaS
screens (steps 2–3) and the OSS wizard (step 4) disappear.

---

## 5. Concrete design ideas to explore (menu — pick/combine)

These are textual prompts for the design agent; not all need to ship.

1. **Two-panel split layout.** Form on one side (~480–560px), a calm brand panel on the other
   (navy field with subtle gold dot grid, the rings motif, tagline, maybe a rotating one-liner
   about durability/reliability). Sharp divider, no shadow. Classic, on-brand, fixes the
   "alone in the void" problem instantly.
2. **Persistent stepper header.** A thin top bar: logo left, horizontal step indicator center
   ("1 Verify · 2 Organisation · 3 Configure"), current step gold, completed steps checked.
   Lives above all onboarding screens including the OSS wizard.
3. **Extend the animated-checklist motif backward.** The OSS setup wizard's reveal/complete
   animation is the best thing we have. Echo its cadence and check-mark language in the earlier
   steps so the whole flow shares one visual rhythm.
4. **Richer org-creation step.** Beyond the name field: a one-line "what's an org" explainer,
   maybe an auto-suggested org name from the email domain, and a reassuring "you can invite
   teammates and rename later."
5. **Warmer empty/verify states.** Replace the bare "Verify your email" card with an
   illustrated, on-brand state that still shows it as step N of M.
6. **Themed PropelAuth pages.** If PropelAuth theming allows: navy/gold, sharp corners, logo,
   Geist font — so the auth jump doesn't look like a different product.
7. **Actionable finish.** Final screen routes into "create your first agent" rather than a
   cold dashboard, closing the loop from sign-up to first value.

---

## 6. Constraints & guardrails for the design agent

- **Stay in Slate.** 0px radius, flat (no shadows), navy + gold + grayscale, Geist fonts, dot
  grid, tight tracking. Reuse existing motion timings.
- **Gold is precious.** Use it for the active step, focus, and the single primary CTA — not as
  a fill.
- **PropelAuth owns the credential form.** Design around the entry and return, and propose
  PropelAuth theme values, but don't assume we can build a fully custom auth form in-app.
- **Email-verification gate must remain** (server-enforced anti-abuse). Design it as a step,
  don't try to remove it.
- **The OSS setup wizard is shared upstream.** Changes there must stay general-purpose (no
  SaaS-only logic in OSS). Prefer enhancing the SaaS-owned screens (steps 0–3) to match the
  wizard rather than forking the wizard.
- **Org name is the only required SaaS-side field.** Don't add required fields without a
  product reason; keep time-to-value short.
- **Responsive.** Two-panel layouts must collapse gracefully to single-column on mobile.

---

## 7. Key files (for whoever implements)

SaaS (where most redesign work lands):
- `apps/saas-ui/src/app/(auth)/layout.tsx`, `(auth)/login/page.tsx`, `(auth)/register/page.tsx`
- `apps/saas-ui/src/app/(onboarding)/layout.tsx`, `(onboarding)/onboarding/page.tsx`
- `apps/saas-ui/src/components/ui/button.tsx` (loading variant), `components/layout/sidebar.tsx`
- `apps/saas-ui/src/app/globals.css` (imports OSS `design-system.css`)

OSS (reference + the nice wizard; modify only general-purpose):
- `apps/ui/src/app/(main)/orgs/[orgId]/setup/page.tsx` (the animated wizard)
- `apps/ui/src/components/onboarding/zero-org-onboarding.tsx`
- `apps/ui/src/app/(auth)/login|register/page.tsx` (on-brand reference forms)
- `apps/ui/src/app/design-system.css`, `apps/ui/DESIGN.md`, `knowledge/ui/brand.md`

---

# Part B — Approved design & implementation plan

The design agent delivered a 6-frame "onboarding arc": **entry gateway → themed
PropelAuth → verify (1/4) → create org (2/4) → provision+configure (3/4) →
done/first-action (4/4)**, unified by a persistent shell (logo header, gold stepper,
navy brand panel) and a session-resolving router. Design approved. The decisions below
resolve the six review questions and fix the premise of the auth screen.

## B0. Verified codebase facts (ground truth for the plan)

- **OAuth email-verify already correct.** `users.email_verified` is synced from
  PropelAuth's `email_confirmed` JWT claim on *every* login, with backend-API fallback
  and monotonic local state (`crates/saas-server/src/auth/propelauth.rs`,
  `sync_user` / `resolve_email_verified`). Google/GitHub users arrive verified. No
  backend change needed — only the UI must treat Verify as conditional.
- **Email infra exists.** OSS `crates/platform/src/email/` (Resend behind `EmailSender`
  trait, `EmailMessage::basic`), already used by org-invite emails
  (`crates/server/src/api/org_invitations.rs`). SaaS `ManagementState` does *not* yet
  hold an `EmailSender` — wiring required. PropelAuth cannot send custom emails.
- **Resume is server-state-driven.** Setup wizard polls org/harness state
  (`apps/ui/src/app/(main)/orgs/[orgId]/setup/page.tsx`); routing keys off `auth/me`.
  Gap: provider setup is skippable, so "has provider" can't mark onboarding done.
- **OSS extension seams exist.** `apps/ui/src/components/onboarding/zero-org-onboarding.tsx`
  takes `usePolicy` + `useCreateOrg` hooks. SaaS currently *duplicates* the onboarding
  page (`apps/saas-ui/src/app/(onboarding)/onboarding/page.tsx`) instead of consuming
  those seams — close this divergence.
- **PropelAuth hosted pages are more capable than first assumed.** The Look & Feel editor
  (ui.propelauth.com) supports **split-screen layout, custom image/gradient backgrounds,
  font customization**, plus logo/colors/dark-mode and custom domains. Fully self-built
  auth UI is *not GA*, and our integration has no password/signup API (SDK is
  redirect-only; backend client = fetch/update/delete/resend user only).

## B1. OSS vs SaaS split

Principle: presentation + general-purpose → OSS; PropelAuth/billing/hosted-only → SaaS.

| Frame / piece | Home | Notes |
|---|---|---|
| Shared **onboarding shell**: header, **gold stepper** (data-driven step list), navy **brand panel** (rings/tagline/feature-checklist) | **OSS** new components | Pure chrome; self-host benefits. Stepper accepts steps[]+currentIndex so it can be 3 or 4 steps. |
| 1 · Entry gateway (`app.everruns.com`, logged-out) | **SaaS** page | Hosted marketing + PropelAuth session resolution. Consumes OSS brand-panel component. Must redirect authenticated users (see B4). |
| 2 · Auth screen | **SaaS / PropelAuth config** | Realized via PropelAuth Look & Feel split-screen (see B-auth). Not React. |
| 3 · Verify email (step 1) | **SaaS** page in OSS shell | PropelAuth concept; conditional on `email_verified` (B2). |
| 4 · Create org (step 2) | **OSS** `ZeroOrgOnboarding`, enhanced | Add explainer + "invite later" card + optional `suggestedName` prop. SaaS injects verify-gate `usePolicy`, PropelAuth `useCreateOrg`, and domain-derived suggestion. Closes the duplication. |
| 5 · Provision+configure (step 3) | **OSS** setup wizard | Restyle to render inside the shell; expand provider catalog/chips. SaaS re-exports. |
| 6 · Done / first action (step 4) | **OSS** | General-purpose post-setup landing; SaaS inherits. Copy must adapt to skipped-vs-configured provider (B5). |

## B-auth. Auth screen decision (Frame 2) — CUSTOM, via OSS-native auth

**Chosen path (updated): build custom in-app auth by adopting the OSS-native `full` auth
backend in SaaS and retiring PropelAuth for credentials.** PropelAuth's API cannot back a
custom login form (no credentials→token endpoint; interactive login is hosted-only), so
"custom" necessarily means using OSS auth, not wrapping PropelAuth. This delivers
pixel-perfect Frame 2 on our own domain in Slate.

Why it's tractable: OSS already ships, production-hardened, in `crates/server/src/auth/`:
- Email+password **login** (`POST /v1/auth/login`) and **signup** (`POST /v1/auth/register`),
  Argon2id hashing (`storage/password.rs`).
- **OAuth Google + GitHub** natively (`auth/oauth.rs`, `/v1/auth/oauth/{p}` + callback).
- **JWT access + single-use refresh rotation** (`auth/jwt.rs`), **PAT** + **CLI** login,
  rate limiting, and a **trait-based `AuthBackend`** so SaaS mounts OSS `BuiltinAuthBackend`
  instead of the PropelAuth backend.
- **Org invitations are already local-DB** (`crates/server/src/api/org_invitations.rs`),
  not PropelAuth — so they are NOT lost by dropping PropelAuth.

Net-new work (all general-purpose → build in OSS, SaaS inherits):
- **Password reset / forgot-password** — DOES NOT EXIST today. Add `POST /v1/auth/forgot-password`
  + `POST /v1/auth/reset-password`, a `password_reset_tokens` migration (hashed token, TTL,
  single-use), email via existing `crates/platform/src/email/` `EmailSender`, and UI pages
  under OSS `apps/ui/src/app/(auth)/`.
- **Email verification flow** — column exists, flow MISSING. Add send-verification +
  verify-token endpoints + email; this becomes our own Frame 3 (we own the email + token),
  replacing reliance on PropelAuth's `email_confirmed`.

Costs / regressions to accept:
- **User migration** (existing SaaS users are all `auth_provider:"propelauth"`,
  `password_hash:None`; PropelAuth=bcrypt, OSS=Argon2id) — see B-migration. OPEN DECISION.
- **MFA/2FA loss** — OSS has none; PropelAuth provided it. Accept or build later.
- **Spec reversal** — contradicts the SaaS PropelAuth integration and
  architecture/access-control knowledge; update those concepts as part of the work.

Open decision — **PropelAuth retirement scope**: (a) fully retire (email+password +
OAuth + remove dep), vs (b) move email+password to OSS now, keep PropelAuth OAuth/MFA
during a transition. Pending user confirmation.

> **RESOLVED (user):** No real production users → clean cutover, **migration DROPPED**
> (PR 6 removed). No PropelAuth MFA reliance → **no MFA replacement** needed. Build the
> full two-panel Frame 2 auth screen **as part of** the cutover (PR 5).
>
> **Cutover is simpler than expected:** the SaaS server already ships a `DevAuthBackend`
> that wraps OSS `BuiltinAuthBackend` (today's PROPELAUTH_URL-empty fallback). PR 5 =
> make that the only path, delete the PropelAuth branch/files, and delete the SaaS
> `(auth)` overrides so login/register fall through to the OSS two-panel pages (PR 5a).

## B-migration. Existing-user migration (DROPPED — no real users)

Every current SaaS user lives in PropelAuth with no local password. Options:
1. **Rehash-on-login (bcrypt→Argon2id):** export PropelAuth hashes, store them, verify
   bcrypt once on first login then transparently rehash to Argon2id. Seamless; needs OSS
   to support a one-time bcrypt verifier + a PropelAuth export. (Recommended if real users
   exist.) OAuth users re-link by email.
2. **Force reset for all:** don't import hashes; flag all users for reset, email a reset
   link at cutover. Simpler; every user resets once.
3. **No real users yet:** dev/early — cut over, handle the few manually.

Pending user confirmation. This choice gates whether OSS needs a transitional bcrypt
verifier.

## B2. Google/OAuth skips verify (review item 2)

Backend already yields `email_verified=true` for OAuth on first sync. Frontend change
only: the stepper is **dynamic** — read `GET /v1/saas/users/me.email_verified`; if true,
do not render Verify as an actionable step (start at Organisation; show a 3-step stepper or
a pre-checked Verify). Password signups still see Verify. Do not hardcode "Step 1 of 4".

## B3. Welcome email after org creation (review item 3)

Wire `email_sender: Arc<dyn EmailSender>` into `ManagementState` (mirror the OSS platform
builder). In `create_organization`, after harness init, send a branded `EmailMessage::basic`
linking to the new org (`{frontend_url}/orgs/{public_id}`), **best-effort** — log on
failure, never fail org creation (mirror the invite-email pattern). Config dependency:
`EMAIL_PROVIDER=resend` + `RESEND_API_KEY` in Doppler (dev + prod). Template/copy is SaaS.

## B4. Don't show onboarding when logged in (review item 4)

Implement the design's routing legend as one server-state resolver keyed off `auth/me`:
- onboarded (org exists && setup complete) → `/chats`
- mid-onboarding → resume the correct step (B6)
- no session → entry gateway → PropelAuth

The **entry gateway must redirect authenticated users away** (it doesn't exist yet).
Consolidate today's scattered guards (root `page.tsx`, `(main)/layout.tsx`, `login/page.tsx`)
into the single resolver so there's one source of truth.

## B5. Messaging fixes (review item 5)

For the design agent — copy that needs changing before build:
- **Verify:** drop "…stays enforced on our side." Use "Verifying your email keeps your
  account secure."
- **Domain auto-suggest:** suppress for free/generic domains (gmail, outlook, yahoo, icloud,
  proton, hotmail…) — never suggest an org named "Gmail." (B-org.)
- **"Encrypted at rest with AES-256":** state AES-256 only if literally true; else
  "encrypted at rest." Verify against provider-credential storage before shipping the claim.
- **Model names** ("GPT-5.5, o-series"): pull from the real catalog or soften; don't ship
  wrong/placeholder names. ("Claude Opus, Sonnet" is fine.)
- **Done screen** "…connected to Anthropic": must be **conditional** — false if the user
  skipped provider setup. Provide skipped-vs-configured variants.
- British "organisation" spelling is used consistently — keep it.

## B6. Drop & return at any step (review item 6)

Resume is server-state-driven already; one gap to close. Because Configure is skippable,
add a durable **org-level `onboarding_completed_at`** (OSS schema + setup page sets it on
finish *or* skip). Resume state machine (off `auth/me` + that flag):

```
!authenticated                          → entry gateway / PropelAuth
authenticated && !email_verified        → Verify
verified && orgs == 0                    → Create organisation
verified && org exists && !completed     → Configure (resume wizard for that org)
verified && onboarded                    → /chats
```

Makes skip durable and every step resumable. Multi-org users are past Create-org; if their
(first) org is incomplete, resume Configure.

## B7. Org-name domain suggestion (supports Frame 4)

Capability in OSS (`ZeroOrgOnboarding` accepts optional `suggestedName`); SaaS computes it
from the PropelAuth email domain, **skipping free/generic providers** (see B5 list).
Suggestion is pre-filled but fully editable; show the "Auto-filled from {domain}" chip only
when a suggestion was actually derived.

## B8. Build order (CONFIRMED: fully retire PropelAuth + rehash-on-login)

User decisions: (1) **fully retire PropelAuth** — SaaS uses OSS-native auth for
email+password AND Google/GitHub OAuth, remove the dependency; accept MFA loss.
(2) **rehash-on-login** migration for existing PropelAuth password users.

PR sequence (each PR-sized, tested, green before the next):

1. **OSS auth gaps (backend)** — password reset + email verification flows. **DONE**
   (commit 13c0b50): endpoints, `088_*` migration, hashed single-use tokens, emails via
   `EmailSender`, enumeration-safe, reset revokes refresh tokens; tests green.
2. **OSS auth gaps (UI)** — `(auth)/forgot-password`, `reset-password`, `verify-email`
   pages + "Forgot password?" link. **DONE** (commit c25804a); build green.
3. **OSS onboarding shell** — shell + data-driven stepper + brand panel. **DONE**
   (commit c9863ff); build green.
4. **OSS onboarding content** — create-org enhancements (explainer, invite-later,
   `suggestedName`), setup wizard restyled into the shell, conditional Done/first-action
   screen, shared step list. **DONE** (commit 437c9a2); build + tests green.
   NOTE: durable `onboarding_completed_at` was split out — see PR 4b below.
4b. **OSS onboarding-complete state + resume** — add durable `onboarding_completed_at`
   (migration + org field; setup Done/skip sets it) and the resume state machine (B6).
   Deferred to design together with the SaaS routing resolver (PR 7).
5. **SaaS cutover to OSS auth** — mount OSS `BuiltinAuthBackend` (`full` mode) in the
   SaaS server instead of the PropelAuth backend; build the custom Slate auth screen
   (Frame 2, two-panel) on our domain; retire PropelAuth provider/SDK from the UI;
   update the SaaS PropelAuth integration and architecture/access-control knowledge.
   ⚠️ CHECKPOINT before starting (see B9 gating question).
6. **User migration (rehash-on-login)** — export PropelAuth bcrypt hashes; add a
   one-time bcrypt verifier in OSS login that, on success, rehashes to Argon2id and
   clears the legacy hash; OAuth users re-link by email. Backfill `email_verified` from
   PropelAuth `email_confirmed` at export so verified users skip Verify.
7. **SaaS onboarding + routing** — consume OSS onboarding via `usePolicy`/`useCreateOrg`
   (retire the duplicated page); entry gateway page (redirects authed users); single
   server-state routing resolver (B4/B6); relocate onboarding/setup out of the sidebar
   shell (see follow-up below).
8. **SaaS welcome email** — wire `EmailSender` into `ManagementState`; send on org
   create, best-effort (B3).
9. **Verify** end-to-end per the drop/return matrix (B6); smoke on dev.everruns.com.

### Follow-ups / known gaps
- **Sidebar on Configure/Done.** The setup route is under `(main)`, so the app sidebar
  still shows on the Configure/Done steps; the design is sidebar-less full-screen.
  Relocate the route (or hide the sidebar during onboarding) as part of PR 7's routing
  work — deferred to avoid a guard/redirect change inside a restyle.
- **verify-email resend needs the email.** Backend builds `/verify-email?token=…` with no
  `email`, so the UI's "resend verification" affordance falls back to a login link.
  Add `&email=` (or a logged-in resend) when wiring SaaS verification.

## B9. Open items / dependencies

- PropelAuth Look & Feel access + custom-domain DNS for `auth.everruns.com`.
- `RESEND_API_KEY` / `EMAIL_PROVIDER=resend` present in dev + prod Doppler.
- Confirm provider-credential encryption wording (AES-256?) before B5 copy.
- `onboarding_completed_at` is an OSS schema change → migration + OSS PR ordering.
- Auth-screen path is the one reversible decision: split-screen now, custom-UI later.
