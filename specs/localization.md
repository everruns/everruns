# Localization And Timezone Resolution

## Abstract

This spec defines how Everruns resolves locale and timezone for backend-authored text, LLM prompt context, and background execution. The goal is consistent behavior across interactive UI turns, scheduled runs, subagents, and other unattended execution paths.

This spec covers:
- durable user/session preferences
- per-turn overrides
- execution-context resolution precedence
- backend localization of system-generated strings

This spec does not define UI localization.

## Goals

1. Keep locale and timezone separate and explicit
2. Make interactive turns use the user's current browser timezone when available
3. Make unattended turns deterministic via durable session and schedule defaults
4. Let messages override locale/timezone for a single turn without mutating durable state
5. Localize backend-authored strings without coupling them to LLM output

## Non-Goals

1. Translating arbitrary stored user content
2. Full ICU/Fluent adoption in the first iteration
3. Inferring timezone from locale
4. Localizing server logs or internal error details

## Terminology

- `locale`: BCP 47 language/region preference such as `uk-UA`
- `timezone`: IANA timezone such as `Europe/Kyiv`
- `execution context`: resolved locale/timezone pair used for one turn or background run
- `interactive turn`: a turn triggered by a live client request
- `background turn`: a turn triggered without a live browser request, for example schedules or resumed work

## Durable State

### Session

Sessions carry durable execution defaults:
- `session.locale`: default language and formatting preference
- `session.timezone`: default timezone for unattended execution and fallback resolution

Behavior:
- At session creation, if the caller provides locale/timezone, store both on the session
- For interactive browser-originated turns, the live request timezone may override the session timezone for that turn
- The control plane should refresh `session.timezone` to the latest browser-provided timezone during interactive use so unattended future runs use the most recent known value

### User Profile

Users carry durable personal defaults:
- `user.locale`
- `user.timezone`

These act as fallbacks when a session has no explicit preference.

### Session Schedule

Schedules already have `schedule.timezone`. This remains distinct from session timezone.

Design rule:
- `schedule.timezone` is authoritative for cron interpretation
- When a schedule fires, its timezone becomes the default execution timezone for that scheduled turn unless a more-specific trigger override is supplied

## Per-Turn Overrides

Message metadata may carry one-turn overrides:
- `message.metadata.locale`
- `message.metadata.timezone`

These values affect only the triggered turn. They do not mutate durable user or session preferences by themselves.

For browser clients, the expected source of truth is:
- locale: explicit client locale if provided, otherwise `Accept-Language` only for default seeding
- timezone: explicit browser timezone sent by the client

Recommended browser contract:
- send current browser timezone explicitly on interactive requests
- do not infer timezone from locale

## Execution Context

Every turn should resolve a single execution context before prompt building, tool execution, and backend-authored messaging:

```rust
pub struct ExecutionContext {
    pub locale: String,
    pub timezone: String,
    pub locale_source: ContextSource,
    pub timezone_source: ContextSource,
}
```

`ContextSource` is an internal enum for observability and debugging, for example:
- `message_override`
- `request`
- `session`
- `user`
- `schedule`
- `integration`
- `default`

The resolved values should be attached to turn-scoped tracing metadata and may be included in event metadata where useful.

## Resolution Rules

### Interactive Turn

#### Locale precedence

1. `message.metadata.locale`
2. explicit request locale
3. `session.locale`
4. `user.locale`
5. locale seeded from `Accept-Language`
6. system default `en`

#### Timezone precedence

1. `message.metadata.timezone`
2. explicit browser/request timezone
3. `session.timezone`
4. `user.timezone`
5. integration/channel timezone if available
6. system default `UTC`

#### Session refresh rule

When an authenticated interactive request supplies a browser timezone:
- use it for the current turn immediately
- update `session.timezone` to that value as the latest known durable timezone

This makes future unattended turns follow the most recently observed user timezone without blocking the current request on inference.

### Scheduled Turn

#### Locale precedence

1. injected message metadata locale
2. `session.locale`
3. `user.locale`
4. system default `en`

#### Timezone precedence

1. injected message metadata timezone
2. `schedule.timezone`
3. `session.timezone`
4. `user.timezone`
5. system default `UTC`

Design rule:
- scheduled turns should not depend on live browser context

### Other Background Turn

Examples:
- resumed durable work
- subagents
- platform-managed background invocations

#### Locale precedence

1. turn/message override
2. `session.locale`
3. `user.locale`
4. system default `en`

#### Timezone precedence

1. turn/message override
2. `session.timezone`
3. `user.timezone`
4. integration/channel timezone if available
5. system default `UTC`

### Subagents

Subagents inherit parent session defaults on spawn:
- `session.locale`
- `session.timezone`

The child may still receive per-turn overrides from later messages.

## Backend Localization

Backend localization applies only to deterministic backend-authored strings:
- cancellation messages
- notification copy
- schedule-triggered inbox text
- tool/system messages authored by the platform
- generic validation and policy error messages exposed to users

It does not apply to:
- LLM-generated content
- logs
- raw internal error messages

### String model

Use stable message keys with parameter interpolation, for example:
- `session.cancel.user_request`
- `session.cancel.agent_response`
- `validation.input_limits_exceeded`
- `schedule.trigger.message`

Recommended API:

```rust
fn render(locale: &str, key: MessageKey, params: &TemplateParams) -> String
```

### Fallback chain

Locale rendering should fall back in this order:
1. exact locale, for example `uk-UA`
2. language-only locale, for example `uk`
3. default locale `en`

### Persistence guidance

When storing backend-authored message text, also persist metadata for traceability:
- `system_message_key`
- `system_message_locale`
- optional interpolation params if needed

This preserves the rendered message while keeping enough context for future analysis or re-rendering.

## API Changes

### Sessions

`POST /v1/sessions`
- accepts optional `locale`
- should accept optional `timezone`

`PATCH /v1/sessions/{session_id}`
- accepts optional `locale`
- should accept optional `timezone`

### Messages

`POST /v1/sessions/{session_id}/messages`
- `metadata.locale` is reserved for per-turn locale override
- `metadata.timezone` is reserved for per-turn timezone override

### Users

`PATCH /v1/users/me`
- should support updating `name`, `locale`, and `timezone`
- patch semantics should allow partial updates

Recommended shape:

```json
{
  "name": "Jane Doe",
  "locale": "uk-UA",
  "timezone": "Europe/Kyiv"
}
```

## Validation

### Locale

- accept normalized BCP 47 tags
- normalize `_` to `-`
- reject empty and malformed tags

### Timezone

- accept only valid IANA timezone names
- reject abbreviations such as `PST` and numeric offsets as durable timezone values
- offsets may still appear in ad hoc tool arguments where appropriate, but not as stored user/session timezone preferences

## Observability

For each turn, emit resolved context in tracing and, when appropriate, event metadata:
- `locale`
- `timezone`
- `locale_source`
- `timezone_source`

This is important for debugging disagreements between browser, session, schedule, and user defaults.

## Security And Trust

- Trust only explicit client-supplied timezone values for browser context
- Do not infer timezone from locale
- Do not persist `Accept-Language` blindly as a durable user preference
- Integration-provided timezone values should be treated as channel context, not as durable user preferences, unless an explicit product decision says otherwise

## Implementation Notes

- Resolve execution context once per turn, then pass it through the worker pipeline
- Keep resolution logic centralized; do not duplicate precedence rules in handlers, tools, and schedulers
- Schedule timezone remains separate even when session timezone exists
- UI localization can later reuse the same user/session preference fields, but the backend contract should not depend on that future work
