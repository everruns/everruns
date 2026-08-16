---
title: ID Schema
description: How Everruns formats and validates public resource identifiers, Stripe-style prefixed IDs.
sidebar:
  order: 40
---

Every resource in the Everruns API, agents, sessions, skills, knowledge bases, and so on, is identified by a **prefixed public ID**. The prefix tells you at a glance what kind of resource you are looking at; the suffix is an opaque 32-character token.

This pattern was popularized by Stripe (`cus_`, `sub_`, `pi_`). Treat the suffix as a meaningless string, do not parse it, sort by it, or infer information from it. The only guarantees the API makes about an ID are its format, its uniqueness within an organization, and its stability over the lifetime of the resource.

## Format

All resource identifiers use the same shape:

```
{prefix}_{32-hex-chars}
```

- `{prefix}` is a short lowercase token that identifies the resource type (for example `agent`, `session`, `skill`).
- `_` separates the prefix from the suffix.
- `{32-hex-chars}` is an opaque 32-character lowercase hexadecimal token.

Example:

```
agent_5c7f3a91b24e48d6a0e91f3b7c4d2e85
```

Identifiers match a fixed pattern: the resource's prefix, an underscore, then exactly 32 lowercase hexadecimal characters. For an `agent`, the literal regex is `^agent_[0-9a-f]{32}$`. The API rejects malformed values with `400 Bad Request`.

## Treat the suffix as opaque

The suffix carries no public meaning. In particular:

- **Do not sort by it.** Use `created_at` (or whatever timestamp field the resource exposes) for chronological ordering.
- **Do not infer age, ordering, or shard placement from it.** The encoding may change without notice.
- **Do not parse it as a UUID.** The hex format is a transport convenience; future resources may use different internal schemes while keeping the same wire format.

This decoupling is intentional. The internal database key for a resource and its public ID are deliberately separate concepts, clients only ever see the public ID.

## Client-Supplied IDs

For resources that accept client-supplied IDs on create, you can pass your own `id` as long as it matches the format above and uses the correct prefix for the resource type. If you omit `id`, the server assigns one.

A subset of resources additionally expose `PUT /v1/{resource}/{id}` as an upsert: the same call creates the resource if it does not exist (`201 Created`) and updates it in place if it does (`200 OK`). Where supported, this makes idempotent provisioning straightforward, replay the same `PUT` and the end state is identical. The OpenAPI reference is the source of truth for which resources support this; not every resource has a `PUT` route.

## Serialization

IDs are always serialized as JSON strings. The field name is `id` for the resource itself and `{resource}_id` when referenced from another resource:

```json
{
  "id": "agent_5c7f3a91b24e48d6a0e91f3b7c4d2e85",
  "session_id": "session_2b8a4d12c673491fae058b7d9c1f6a40"
}
```

## Prefix Reference

The prefix is part of the public contract for each resource. The most common ones are listed below; the canonical list lives in the OpenAPI specification.

The prefix in the table below is the token that appears before the `_` separator, an `agent` resource has IDs that start with `agent_`.

| Resource | Prefix |
|----------|--------|
| Agent | `agent` |
| Agent version | `agentver` |
| Session | `session` |
| Skill | `skill` |
| Knowledge base | `kb` |
| Memory | `mem` |
| MCP server | `mcp` |
| Schedule | `sched` |
| Image | `img` |
| User | `user` |
| Organization | `org` |

## Design Notes

| Question | Answer |
|----------|--------|
| Why prefixed IDs? | They make IDs self-describing and prevent accidentally passing, say, an agent ID where a session ID is expected. |
| Why opaque suffixes? | Decoupling the wire format from internal storage gives the platform room to evolve without breaking clients, and prevents callers from leaning on accidental properties (ordering, creation time) that aren't part of the contract. |
| Why lowercase hex? | Case-insensitive matching, URL-safe, easy to copy and paste. |
| Are IDs unique across organizations? | Each ID is unique within its owning organization. The same `id` value could in principle appear in two different orgs, but you only ever see IDs scoped to orgs you belong to. |
