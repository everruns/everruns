---
type: Decision
title: "Bound Secrets"
description: "Scoped write-only credential bindings, secure HTTP injection and response capture, durable consent, and per-request revocation."
tags:
  - everruns
  - security
  - secrets
  - bashkit
---
# Bound Secrets

Status: **Implemented; controlled HTTPS capture and reuse qualified.**

A bound credential combines an encrypted write-only value with a destination
and placement. The model uses `$NAME`; the host substitutes its value only at
the authorized transport boundary. Ordinary capability-owned session secrets
remain separate. Memory ownership and storage are excluded from this change.

## Ownership and authority

[Resource access](resource-access.md) owns the common contract: tenant, typed
owner, verified invocation, explicit grants, and revocation. Credential owners
may be a session, agent, virtual user, or organization. User and service usage
are explicit; there is no scope-precedence or identity-fallback chain. Multiple
authorized bindings with the same reference name fail as ambiguous.

Personal use requires the persisted invocation's verified virtual user in a
private Console/API transcript and a workspace used by one session. Channel/App
delivery surfaces have no proved private audience and refuse personal use;
explicit service grants remain available. It cannot borrow the session owner, a previous
speaker, or administrative impersonation. Agent credentials follow the actual
active responder. Organization ownership grants no implicit runtime access.
A use grant never permits management or resharing.

## One binding domain

`credential_bindings` replaces `agent_mcp_secret_bindings` and owns both HTTP
and MCP parameter values. Typed shared access headers and grants have tenant
foreign keys. Metadata APIs never return encrypted or plaintext values.
The shared command tree exposes metadata declaration, listing, revocation and
explicit grant administration with worked CLI examples. It never accepts a bound
credential value; secure entry stays on the dedicated authenticated HTTP form.

Legacy agent APIs adapt to this domain. Bound `session_secrets` migrate with
stable IDs and encrypted bytes to session-wide service bindings, preserving
their original explicit session lifetime. No personal or durable agent grant
is inferred. Capability-owned unbound secrets stay in `session_secrets`.
Old workers cannot write bound values to that retired store or resolve values
using only a session ID; server and workers must upgrade together.

HTTP bindings allow HTTPS origin plus path prefix, bearer authorization, a
named header, or a named form field. URL matching is structural and path-bound;
encoded separators/dot segments and transport-owned headers fail closed.
References in URLs, JSON, arbitrary bodies, and mixed field strings are refused.
Form values are encoded only after substitution. Bindings never widen egress.

MCP bindings name an exact server endpoint, tool, and top-level parameter. The
model-visible schema omits the bound parameter. The server authorizes and
injects immediately before sending, then redacts known values from results.

## HTTP capture and consent

[Secure web_fetch credentials](../execution/web-fetch-credentials-request.md)
owns detailed acceptance requirements; implementation and exported schemas own
field shapes. A request may capture up to eight declared JSON-pointer fields.
The host reads a complete bounded response, rejects duplicate JSON keys and
invalid/missing captures, extracts all values, replaces their fields with
references, and redacts known literal, JSON-escaped, and form-encoded echoes.
Only then may a result reach hooks, events, output files, or the model.

All captures and the sanitized result commit atomically. Failed validation or
persistence withholds the entire response. Authentication polling returns only
allowlisted error codes; pending/slow-down responses can retry the same approved
plan. An uncertain already-dispatched request is never automatically replayed.

Persistent capture, replacing a value, or a cross-origin destination requires
durable, expiring, single-use consent before dispatch. Consent binds the exact
request, owner, names, destinations, placements, and expiry extraction. The UI
shows those facts and offers approve/decline only. Ordinary always-allow rules
cannot authorize capture. Approval requires the authenticated owner-management
policy; a runtime or unrelated participant cannot consent for a personal owner.

## Runtime and lifecycle

Bashkit receives fresh opaque environment references, never values. Its egress
adapter resolves authorization at **each HTTP send**, including a long shell
call. Header/form validation and bounded response redaction run before any
response bytes enter the interpreter. Normal requests retain their existing
transport and output behavior. `web_fetch` uses the same credential service,
URL matcher, and redactor; capture cannot crawl, render, extract, or save files.
Credential responses cannot redirect secrets to another destination.

Secure entry uses a masked field with no readback or prefill. Forms clear on
success, failure, cancellation, or owner/organization change. Revision checks
bind rotation to the reviewed target; revocation cannot resurrect a stale
capture. Expiry blocks use with renewal guidance. Settings shows expiry and
recent use; org/agent/session and virtual-user surfaces use the same panel.
Credential/grant changes have atomic payload-free access audit entries.

Connection-backed bindings retain a reference rather than copying OAuth tokens.
Provider adapters must enforce audience and narrowing. GitHub App HTTP bindings
require the native selected service identity, explicit repository/permission
limits, `api.github.com`, and installation-scope evidence. Personal tokens cannot
satisfy restricted exposure. GitHub's existing App/person selection, setup,
refresh, and verified inbound event routing remain provider-owned.

## Limits

Credential use does not change the organization’s existing session-history
management permissions. Private execution excludes other runtime participants
and shared workspaces; transcript/export governance remains owned by session
policies.

Revocation blocks subsequent authorized sends; it cannot recall requests already
sent or data already disclosed. Redaction covers declared captured values and
known injected literals/JSON/form encodings. Arbitrary reversible transforms
such as base64 are outside that guarantee. Every expected credential response
field must be declared; a host cannot infer unknown token fields reliably.

Threats: TM-BASH-030/031/032/033 and TM-AGENT-038 in the
[threat model](threat-model.md). The common resource contract owns connection
access; this decision does not migrate memory authorization.

## Qualification

Migration and authorization tests cover preserved encrypted values, exact
subjects/responders, consent replay, stale revisions, provider narrowing and
per-send revocation. [UI acceptance](../test-cases/ui/agent_credentials/TC004_scoped_credentials_and_capture.md)
covers all four owner surfaces and exact-request consent. A disposable HTTPS
response was captured through the live approval UI, reused across sessions,
agents and Bashkit, and revoked; events, exports, persisted output and logs
contained no captured value.

The real Shipmail account acceptance remains blocked on the handoff's auth.md
URL and authorized account context. Controlled-fixture qualification does not
claim completion of that provider account flow.
