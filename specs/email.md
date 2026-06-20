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

`everruns-core` owns the email abstraction:

- `EmailSender` is the async trait used by internal callers.
- `EmailMessage` is the provider-neutral request shape.
- `SentEmail` is the provider-neutral success result.
- `EmailError` separates configuration, request validation, transport, and provider failures.
- Platform/application state stores the sender as `Arc<dyn EmailSender>` when it must be shared across services.
- `PlatformDefinition` carries the active system email sender as part of the platform profile.
- Concrete senders use `EgressService` for provider HTTP; email remains a
  domain-specific service above the outbound transport boundary.

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

All templates share an app-styled HTML shell whose colors and shapes mirror `apps/ui/src/app/design-system.css` (grayscale surface, navy/gold brand accents, sharp 0px corners). Brand tokens are inlined as literals in `crates/core/src/email.rs` because email clients cannot load the app's webfont or CSS variables; keep them in sync if the palette changes.

- `EmailTemplate::Minimal(MinimalEmailTemplate)`
  - app-styled shell, but **unbranded**: no wordmark, logo, or footer link
  - plain text body is passed through verbatim
  - use when the surrounding flow already carries Everruns branding
- `EmailTemplate::Basic(BasicEmailTemplate)`
  - same app-styled shell as `Minimal`, plus branding
  - header with the Everruns logo (`https://everruns.com/logo-64.png`) and wordmark, linking to everruns.com
  - footer linking back to everruns.com
  - plain text body is prefixed with the Everruns wordmark and gets a site-link footer

The logo is referenced as a hosted absolute PNG URL rather than the in-repo SVG, because most email clients block SVG and inline data URIs. The marketing site serves `logo-64.png` at the apex domain — a 64px PNG sized for the 28px header at 2x retina.

## System Configuration

Email delivery is system-wide and configured from process environment:

- `EMAIL_PROVIDER=disabled|resend`
- `RESEND_API_KEY`
- `RESEND_API_BASE_URL` for tests or controlled endpoint overrides

When `EMAIL_PROVIDER` is unset, the system email configuration is disabled. Production deployments that send email must set `EMAIL_PROVIDER=resend` and provide the Resend variables.

The default OSS platform profile resolves `SystemEmailConfig::from_env()` during `oss_platform_definition_for_grade()` construction. Invalid email configuration fails startup when a provider is explicitly enabled.

Embedders can bypass env-based setup by constructing a custom `PlatformDefinition` and calling `PlatformDefinition::builder().email_sender(...)` before passing it to `ServerAppBuilder::platform_definition(...)`.

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
