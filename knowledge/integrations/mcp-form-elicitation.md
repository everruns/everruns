---
type: Specification
title: "Inbound Form Mode Elicitation"
description: "Answering an attached MCP server's form mode elicitation through ask_user, and the trust rules that shape it."
tags:
  - everruns
  - integrations
  - security
---
# Inbound Form Mode Elicitation

## Summary

Everruns' MCP client declines form mode `elicitation/create` outright. Only URL
mode is handled ([MCP Server Specification](mcp-servers.md) § URL mode
elicitation). This specification defines the inbound half: an attached server's
`requestedSchema` becomes an `ask_user` question set, the session's own surface
collects the answer, and the answer returns to the server as an `ElicitResult`.

It is the inverse of the outbound projection Everruns already ships as an MCP
*server* (`crates/server/src/api/mcp_endpoint/form_elicitation.rs`), which maps
an `ask_user` question set onto MCP's restricted schema. That direction trusts
the question, because Everruns wrote it. This direction does not.

Nothing in this document is implemented yet. It exists because EVE-1068's
remaining blocker was a security decision, not a mapping problem, and the
decision has to be reviewable before the code is written.

## Why this needed its own pass

Two threats decide the shape, and together they are why form mode was left
undeclared when URL mode was built.

**The question is third-party authored and renders in Everruns' chrome.** A
server that elicits "confirm your password to continue" gets Everruns'
credibility behind the prompt. URL mode's analogous problem — a third party
asking a person to open a link — is answered with domain highlighting and
Punycode warnings (TM-TOOL-033). Form mode has no domain to show, so it needs a
different answer.

**The answer routes back through the model.** `crates/mcp/src/protocol.rs`
records this as the reason form mode stays undeclared: a form answer is carried
by a tool result, so it lands in the event log and permanently in model context.
That is the TM-AGENT-016 class of problem, and it is exactly what URL mode exists
to avoid.

The second threat is the sharper one, and it forces a conclusion the issue did
not anticipate: **`ask_user`'s `Secret` kind cannot serve this path.** A `Secret`
answer carries a `session:{name}` reference and deliberately has no field for a
value. A remote server eliciting a credential wants the value. So the only
available behaviors are to refuse, or to leak. This specification refuses, and
says so in a way the server can act on.

## Decisions

### D1. Default-deny, per-server opt-in

Form mode is declared to a server only when that server's configuration opts in.
This is the per-server elicitation policy [mcp-servers.md](mcp-servers.md)
records as "not yet built"; form mode is a stronger reason to want it than URL
mode was, because URL mode never puts server-authored text in front of a person
as a question Everruns appears to be asking.

- A new per-server `elicitation_policy` field, alongside `protocol_mode` on both
  `McpServer` and `ScopedMcpServer`, defaulting to the current behavior so
  existing configuration is byte-identical when serialized.
- `url` (the default) declares URL mode only, which is today's behavior.
- `url_and_form` additionally declares form mode.
- `none` declares neither, which is the only way an operator can currently stop
  a configured server from eliciting at all.
- The policy is an operator decision on the server record, never a per-call
  negotiation and never something the model can widen.

### D2. Attribution is mandatory and is not server-authored

Every question projected from an inbound elicitation names the server that asked,
in a frame Everruns owns:

- The question's `header` is composed by Everruns from the server's configured
  name and never from schema-supplied strings. A server cannot author the line
  that establishes who is asking.
- Server-supplied `title` and `description` appear only in body positions, as the
  question text and option descriptions — the same positions that already carry
  untrusted tool output.
- The surface states that the question comes from an external server and that
  Everruns is not asking it. This is the form-mode counterpart of URL mode's
  highlighted domain.
- Server strings are rendered as text, never as markup or links. The existing
  card escaping rules apply (TM-MCP-003 is the same class on the outbound side).

### D3. Credential-shaped properties are refused, not collected

A property is credential-shaped when any of these hold:

- `format: "password"`, or `writeOnly: true`
- the property name or title matches the credential vocabulary already used by
  `crates/server/src/credential_shape.rs`

Such a property makes the whole elicitation unanswerable. The client answers
`decline` and fails the call with an error naming the property and pointing at
URL mode, which is the mechanism the MCP specification provides for exactly this
("a server that needs a secret, a third-party authorization, or a payment must
not ask the client for it").

Mapping these to `Secret` is rejected on purpose. It would look safe while being
unimplementable: the `secret_ref` a `Secret` answer carries is meaningless to the
remote server, so honouring the elicitation would mean putting the plaintext in
`ElicitResult.content`, hence in the tool result, hence in the event log and
model context forever.

### D4. The mapping, and what stays out of profile

`requestedSchema` is a restricted profile: one object, one level of properties,
each a primitive or an enum of them. Each property becomes one question.

| Schema property | Question | Notes |
|---|---|---|
| `string` with `enum` | `Choice`, `allow_other: false` | `enumNames` supplies labels when present, values otherwise |
| `string` | `Text` | |
| `boolean` | `Choice` over Yes/No | `default` selects the default option |
| `number`, `integer` | `Text`, with the type and any bound stated in the question text | Re-validated on the way back; a non-conforming answer is a re-ask, not a coerced value |
| `string` with `format` other than `password` | `Text`, with the format stated in the question text | Not enforced client-side; the server validates |
| anything else (arrays, nested objects, `$ref`) | refused | Out of profile; the call fails naming the property |

Multi-select has no inbound counterpart: the profile has no array, and the
outbound side's one-boolean-per-option encoding is a convention Everruns chose
for itself and cannot assume of a server.

Bounds, because a schema is attacker-controlled input (TM-DOS class): at most 16
properties, at most 32 enum members per property, and property names, titles and
descriptions truncated to the limits the `ask_user` contract already enforces. A
schema exceeding any bound is refused rather than trimmed, so a person is never
shown a silently shortened question.

### D5. Unanswered resolves `decline`, never an empty `accept`

This inherits EVE-1096's rule for questions with no default-able answer. A
`Text` question has no default, and neither does a question nobody answered:

- An unattended run, a timeout, or a dismissal returns
  `ElicitResult { action: "decline" }`.
- `accept` is sent only with content for every required property.
- An empty or partial `accept` is never sent. A server reading an empty string as
  an answer is worse than any decline.

### D6. The credential-shape check runs on the way out

EVE-1059's deterministic credential-shape check already refuses
credential-shaped free-text answers at
`crates/server/src/api/question_answers.rs`. Because every inbound form answer is
free text or a server-supplied label, that check is on the path by construction —
but the decline must be reported to the *server* as a decline, not surfaced as an
Everruns error, so the turn ends cleanly.

### D7. Declared only when answerable

Same rule as URL mode, and the same mechanism: the host injects a form handler,
and a host that injects none leaves the capability undeclared, so a compliant
server cannot ask. Unattended workers inject nothing.

`_meta` therefore carries `{"elicitation": {"url": {}, "form": {}}}` only when
both a handler exists and the server's `elicitation_policy` allows form mode.
This is 2026-07-28 only, for the same reason URL mode is.

## Threat model

Recorded as TM-TOOL-043 through TM-TOOL-047 in
[threat-model.md](../security/threat-model.md). All five are **OPEN** until the
implementation lands, because the current decline is not a mitigation of these
threats — it is the absence of the feature that creates them.

## How the session hosts answer

The pause mechanics are URL mode's, reused rather than rebuilt: the executor
returns a structured tool result, an act hook recognises it and parks the session,
and the answer arrives through the existing `ask_user` question-set machinery
([ask-user.md](../execution/ask-user.md)) rather than a new surface. The
difference from URL mode is that the answer is data the server consumes, so the
retry carries `inputResponses: {<key>: {action, content}}` rather than a bare
`accept`.

Because the retry may run in a different worker process than the call that asked,
the collected answer is session-scoped durable state, as URL mode's consent
record is.

## Acceptance

- A server with `elicitation_policy: url` (the default) or no policy sees no
  `form` capability and cannot elicit a form. Today's behavior is unchanged.
- An opted-in server's flat primitive schema reaches a person as an attributed
  `ask_user` question set, and their answer reaches the server as `accept` with
  content.
- A credential-shaped property is declined with an error naming the property and
  URL mode, and nothing credential-shaped is ever written to the event log by
  this path.
- An out-of-profile or over-bound schema is refused, naming the property.
- An unattended run declines rather than parking or answering empty.
- The question surface states which server asked, and no server-supplied string
  can occupy the attribution line.
