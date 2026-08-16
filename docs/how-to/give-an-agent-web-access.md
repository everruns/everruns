---
title: Give an agent web access
description: Enable the web_fetch capability, restrict outbound network access with allow/block lists, and verify the agent reaches only intended hosts.
---

`web_fetch` gives an agent the `web_fetch` tool, fetch any URL, optionally convert HTML to markdown. By default it can reach any public host, with built-in SSRF protection blocking private IPs. To restrict it further, layer **network access lists** on the harness, agent, or session.

## Enable the capability

```bash
curl -X PATCH http://localhost:9300/api/v1/agents/$AGENT_ID \
  -H "Content-Type: application/json" \
  -d '{
    "capabilities": [
      { "ref": "web_fetch" }
    ]
  }'
```

Or in an agent definition file:

```yaml
capabilities:
  - ref: web_fetch
  - ref: session_file_system  # so the agent can save fetched content
```

## Restrict to specific hosts

Pass `network_access` when creating or updating the agent. Patterns can be exact domains, wildcard domains, or URL prefixes:

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Research Agent",
    "system_prompt": "You are a research assistant.",
    "capabilities": [{ "ref": "web_fetch" }],
    "network_access": {
      "allowed": ["*.github.com", "api.openai.com", "https://docs.python.org/3/"],
      "blocked": ["evil.example.com"]
    }
  }'
```

| Pattern | Matches |
|---|---|
| `api.example.com` | Exact domain |
| `*.example.com` | Domain + all subdomains |
| `https://api.example.com/v1/` | URL prefix |

Domain matching is case-insensitive. **Blocked patterns always win** over allowed patterns.

## Layered policies

The three layers (harness → agent → session) can only narrow access, never expand it:

- `allowed` lists intersect across layers.
- `blocked` lists union across layers.

So a session can tighten what its agent allows but cannot punch a hole through the agent's `blocked` list.

## Tighten further per-session

When you don't trust a particular session's input, restrict more:

```bash
curl -X POST http://localhost:9300/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "agent_id": "agent_...",
    "network_access": {
      "blocked": ["internal.corp", "*.staging.example.com"]
    }
  }'
```

## SSRF protection

Even without an explicit policy, `web_fetch` blocks private IP ranges by default, loopback, RFC1918, link-local, and CGNAT, with DNS pinning to prevent rebinding attacks. To explicitly allow a private host, set it in `allowed` and disable SSRF protection at the harness level (see [Network access control](/advanced/network-access/)).

## Verify

Try a URL the agent should reach, then one it shouldn't, and check the tool result events:

```python
await client.messages.create(session.id, "Fetch https://api.github.com/zen")
# Should succeed.

await client.messages.create(session.id, "Fetch https://evil.example.com/")
# Should fail with a network policy error in the tool result.
```

## See also

- [Network access control](/advanced/network-access/), full pattern semantics and layering rules.
- [Web Fetch capability](/capabilities/web-fetch/), tool reference.
- [Equip an agent with tools](/how-to/equip-agents-with-tools/)
