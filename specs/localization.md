# Localization And Timezone Resolution

## Abstract

This spec defines how Everruns resolves locale and timezone for backend-authored text, LLM prompt context, and background execution. The goal is consistent behavior across interactive UI turns, scheduled runs, subagents, and other unattended execution paths.

This spec covers:
- durable user/session preferences
- per-turn overrides
- execution-context resolution precedence
- backend localization of system-generated strings

This spec also defines the first iteration of UI localization plumbing and
shared string-catalog rules so new locales do not require hard-coded strings
throughout the product.

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

Message controls may carry one-turn overrides:
- `message.controls.locale`
- `message.metadata.locale` (legacy/reserved compatibility path)
- `message.metadata.timezone`

These values affect only the triggered turn. They do not mutate durable user or session preferences by themselves.

For browser clients, the expected source of truth is:
- locale: explicit client locale in controls if provided, otherwise `Accept-Language` only for default seeding
- timezone: explicit browser timezone sent by the client

Recommended browser contract:
- send current browser locale explicitly in `controls.locale` on interactive requests
- seed `session.locale` from the browser locale when creating browser-originated sessions
- allow per-message `controls.locale` to override session locale without mutating the session
- message controls win over session defaults for a single turn
- message metadata locale remains reserved for compatibility and internal callers
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

Execution context resolution follows a single precedence chain. The first non-null value wins. Subagents inherit parent session defaults on spawn.

### Locale Precedence

| Priority | Interactive Turn | Scheduled Turn | Background Turn |
|----------|-----------------|----------------|-----------------|
| 1 | `message.controls.locale` | injected message metadata locale | turn/message override |
| 2 | `message.metadata.locale` | `session.locale` | `session.locale` |
| 3 | explicit request locale | `user.locale` | `user.locale` |
| 4 | `session.locale` | system default `en` | system default `en` |
| 5 | `user.locale` | | |
| 6 | seeded from `Accept-Language` | | |
| 7 | system default `en` | | |

### Timezone Precedence

| Priority | Interactive Turn | Scheduled Turn | Background Turn |
|----------|-----------------|----------------|-----------------|
| 1 | `message.metadata.timezone` | injected message metadata timezone | turn/message override |
| 2 | explicit browser/request timezone | `schedule.timezone` | `session.timezone` |
| 3 | `session.timezone` | `session.timezone` | `user.timezone` |
| 4 | `user.timezone` | `user.timezone` | integration/channel timezone |
| 5 | integration/channel timezone | system default `UTC` | system default `UTC` |
| 6 | system default `UTC` | | |

### Session Refresh Rule

When an authenticated interactive request supplies a browser timezone, use it for the current turn immediately and update `session.timezone` so future unattended turns follow the most recently observed user timezone.

### Design Rules

- Scheduled turns should not depend on live browser context
- Subagents inherit `session.locale` and `session.timezone` from parent on spawn; per-turn overrides still apply

## Backend Localization

Backend localization applies only to deterministic backend-authored strings:
- cancellation messages
- notification copy
- schedule-triggered inbox text
- tool/system messages authored by the platform
- generic validation and policy error messages exposed to users
- tool display names, narration, and activity headlines emitted in events

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
- `tool.label.read_file`
- `tool.narration.read.started`
- `chat.empty_state.title`

Implementation rule:
- UI strings and backend-authored strings must live in locale catalogs rather than
  inline string literals inside rendering/business-logic code
- catalogs may stay code-native in the first iteration; Fluent/ICU are not required yet
- adding a new locale should primarily mean adding catalog entries, not rewriting logic

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
- `controls.locale` is the preferred per-turn locale override
- `metadata.locale` is reserved for per-turn locale override
- `metadata.timezone` is reserved for per-turn timezone override

`POST /v1/sessions/chat`
- may accept optional `locale` for seeding the singleton global chat session on first creation

## UI Localization

### Browser locale detection

- The browser client should detect locale from `navigator.languages` / `navigator.language`
- UI copy should switch by supported language family (`uk*` → Ukrainian, otherwise English in the first iteration)
- The raw normalized browser locale (for example `uk-UA`) should still be forwarded to the backend so later locales can be enabled without changing the transport

### Catalog structure

- Keep a typed UI string catalog keyed by stable message IDs
- Keep a backend string catalog keyed by stable message IDs or semantic helpers
- Use the same fallback rule everywhere:
  1. exact locale if present
  2. language family if supported
  3. default English

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

- Resolve execution context once per turn, then pass it through the worker pipeline -- keep resolution logic centralized
