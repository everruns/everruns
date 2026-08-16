---
type: Specification
title: "Email Sending"
description: "Internal email delivery abstraction."
tags:
  - everruns
  - operations
---
# Email Sending

## Intent

Provide an internal abstraction for system-owned email delivery.

Email is not an agent capability, public API, or UI surface. It is a host service used by product and operational flows that need to send user-facing transactional email.

## Design Goals

1. Keep call sites provider-neutral.
2. Keep provider credentials out of agent/session/user configuration.
3. Support one system-wide sender configuration per deployment.
4. Leave room for future concrete implementations such as SendGrid, Cloudflare, SES, SMTP, or provider-specific SDKs.

## Core Contract

`everruns-platform` owns the email abstraction (moved out of `everruns-core`
in EVE-879, email is a hosted product side effect, never consumed during a
turn, so the execution kernel carries none of it):

- `EmailSender` is the async trait used by internal callers.
- `EmailMessage` is the provider-neutral request shape.
- `SentEmail` is the provider-neutral success result.
- `EmailError` separates configuration, request validation, transport, and provider failures.
- Platform/application state stores the sender as `Arc<dyn EmailSender>` when it must be shared across services.
- The server composes the active system email sender on `ServerAppBuilder`
  (`email_sender`, defaulting to the environment-configured
  `SystemEmailConfig`); it is not part of the execution-facing
  `HostComposition`.
- Concrete senders use direct provider HTTP clients; email remains a
  domain-specific host service outside the tenant/agent egress boundary.
- System email is not governed by tenant/agent egress policy such as
  `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`.

Messages support:

- fixed system sender: `no-replay@everruns.com` (current verified sender identity)
- `to`
- subject
- template-rendered HTML and plain text body
- provider-neutral tags
- optional idempotency key

At least one `to` recipient, a subject, and a valid template are required.

### Templates

Email sends use explicit templates. Callers select a template by constructing the matching `EmailTemplate` variant (or its `EmailMessage` constructor); there is no runtime template registry. Both templates accept a caller-supplied `text` and `html` body and require both to be non-empty.

All templates share an app-styled HTML shell whose colors and shapes mirror `apps/ui/src/app/design-system.css` (grayscale surface, navy/gold brand accents, sharp 0px corners). Brand tokens are inlined as literals in `crates/platform/src/email/mod.rs` because email clients cannot load the app's webfont or CSS variables; keep them in sync if the palette changes.

- `EmailTemplate::Minimal(MinimalEmailTemplate)`
  - app-styled shell, but **unbranded**: no wordmark, logo, or footer link
  - plain text body is passed through verbatim
  - use when the surrounding flow already carries Everruns branding
- `EmailTemplate::Basic(BasicEmailTemplate)`
  - same app-styled shell as `Minimal`, plus branding
  - header with an inline Everruns logo mark and wordmark linking to everruns.com
  - footer linking back to everruns.com
  - plain text body is prefixed with the Everruns wordmark and gets a site-link footer

Templates must not embed remote images by default. Transactional email is used by self-hosted deployments and auth/onboarding flows, so opening a message must not cause a mail client to fetch upstream marketing-domain assets that can reveal recipient IP address, user-agent, or open timing.

## System Configuration

Email delivery is system-wide and configured from process environment:

- `EMAIL_PROVIDER=disabled|resend`
- `RESEND_API_KEY`
- `RESEND_API_BASE_URL` for tests or controlled endpoint overrides

When `EMAIL_PROVIDER` is unset, the system email configuration is disabled. Production deployments that send email must set `EMAIL_PROVIDER=resend` and provide the Resend variables.

The default OSS server composition resolves `SystemEmailConfig::from_env()` via `crate::platform::system_email_sender()` when `ServerAppBuilder` runs without an explicit sender. Invalid email configuration fails startup when a provider is explicitly enabled.

Embedders can bypass env-based setup by calling `ServerAppBuilder::email_sender(...)` with their own `Arc<dyn EmailSender>` (EVE-879). Reusable OSS helpers include `SystemEmailConfig::into_sender()`, `ResendEmailSender::new(...)`, and `ResendEmailSender::with_http_client(...)` for deployments that assemble their own sender.

`ServerContext.email_sender` exposes the configured sender to internal background tasks and extension code. Request handlers should access the sender through their service/application state when a feature needs transactional email.

## Resend Implementation

The first concrete implementation is `ResendEmailSender`.

Provider behavior:

- Sends mail through Resend `POST /emails`.
- Always sends from `Everruns <no-replay@everruns.com>`.
- Uses the `Idempotency-Key` header when `EmailMessage.idempotency_key` is set.

The Resend API key is a deployment secret. It must be injected by the deployment environment, not stored in repo or database records. The Resend account must have `everruns.com` verified for sending.

## Non-Goals

- No user-facing email settings UI.
- No REST API endpoints for ad hoc email sending.
- No agent capability or tool for sending email.
- No per-organization sender configuration yet.
- No durable outbound email queue yet.
