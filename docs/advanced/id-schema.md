---
title: ID Schema
description: How Everruns formats and validates public resource identifiers (Stripe-style prefixed IDs backed by UUIDv7)
sidebar:
  order: 40
---

Every resource in the Everruns API — agents, sessions, skills, knowledge bases, and so on — is identified by a **prefixed public ID**. The prefix tells you at a glance what kind of resource you are looking at; the suffix is a UUIDv7 rendered as 32 lowercase hex characters with no dashes.

This pattern was popularized by Stripe (`cus_`, `sub_`, `pi_`) and formalized by the [TypeID spec](https://github.com/jetpack-io/typeid). Everruns uses hex encoding (32 characters) instead of TypeID's base32 (26 characters) for simpler debugging and direct UUID compatibility.

## Format

All resource identifiers use the same shape:

```
{prefix}_{32-hex-chars}
```

- `{prefix}` is a short lowercase token that identifies the resource type (for example `agent`, `session`, `skill`).
- `_` separates the prefix from the suffix.
- `{32-hex-chars}` is a UUIDv7 serialized as lowercase hex with no dashes.

Example:

```
agent_01933b5a00007000800000000000001
```

Identifiers must match the regular expression `^{prefix}_[0-9a-f]{32}$`. The API rejects malformed values with `400 Bad Request`.

## Why UUIDv7?

UUIDv7 embeds a millisecond timestamp in its leading bits, so IDs sort roughly in creation order. That makes paginated listings stable, makes log lines easier to scan chronologically, and avoids the index fragmentation that random UUIDv4 keys cause.

## Client-Supplied IDs and Upsert

You can supply your own `id` when creating a resource as long as it matches the format above and uses the correct prefix for the resource type. If you omit `id`, the server generates one.

Because IDs are stable and unique per organization, `PUT /v1/{resource}/{id}` performs an upsert:

- If no resource with that ID exists, it is created (`201 Created`).
- If one already exists, it is updated in place (`200 OK`).

This makes idempotent provisioning straightforward — replay the same `PUT` and the end state is identical.

## Serialization

IDs are always serialized as JSON strings. The field name is `id` for the resource itself and `{resource}_id` when referenced from another resource:

```json
{
  "id": "agent_01933b5a00007000800000000000001",
  "session_id": "session_01933b5a00007000800000000000003"
}
```

## Prefix Reference

The prefix is part of the public contract for each resource. The most common ones are listed below; the canonical list lives in the OpenAPI specification.

| Resource | Prefix |
|----------|--------|
| Agent | `agent_` |
| Agent version | `agentver_` |
| Session | `session_` |
| Skill | `skill_` |
| Knowledge base | `kb_` |
| Volume | `vol_` |
| MCP server | `mcp_` |
| Schedule | `sched_` |
| Image | `img_` |
| User | `user_` |
| Organization | `org_` |

## Design Notes

| Question | Answer |
|----------|--------|
| Why prefixed IDs? | They make IDs self-describing and prevent accidentally passing, say, an agent ID where a session ID is expected. |
| Why UUIDv7? | Time-ordered, globally unique, index-friendly. |
| Why lowercase hex? | Case-insensitive matching, URL-safe, easy to copy and paste. |
| Are IDs unique across organizations? | Each ID is unique within its owning organization. The same `id` value could in principle appear in two different orgs, but you only ever see IDs scoped to orgs you belong to. |
