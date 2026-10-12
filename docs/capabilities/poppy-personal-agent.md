---
title: Poppy Personal Agent
description: Let an agent talk to companies' agents for the person it works for, over the Personal Agent Protocol (Poppy).
sidebar:
  order: 98
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `poppy_client` |
| **Category** | Integrations |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Medium |

The [Personal Agent Protocol](https://personalagentprotocol.org) ("Poppy",
draft 0.1) lets a person's own agent do business with a company: find it, start
a conversation with its agent, and hand off to a person there. With this
capability an Everruns agent is that personal agent. Any company that publishes
`/.well-known/poppy.json` on its domain works, with nothing to register.

## When to enable

- A personal assistant such as Platform Chat, where people ask the agent to
  check on an order, a booking or a support case with a company.

Leave it off for agents that should not contact outside companies.

## Configuration

None.

```json
{ "ref": "poppy_client", "config": {} }
```

## Tools

| Tool | What it does |
|---|---|
| `poppy_discover` | Looks up a company by domain and says what it offers |
| `poppy_send` | Sends a message to the company's agent and waits briefly for the reply. The first message starts the conversation; `new_conversation` starts another |
| `poppy_read` | Reads replies that arrived since the last answer |
| `poppy_handoff` | Asks the company for a person |
| `poppy_close` | Ends the conversation |

## Identity and privacy

Every company sees the deployment as one personal agent named "Everruns". Its
client metadata is published at `/v1/poppy/agent.json` and its public key at
`/v1/poppy/jwks.json`, under the API base URL. Each person gets a different
random User ID at each company, so two companies cannot tell they talk to the
same person. Nothing else about the person, the organization or the
conversation is sent, beyond the messages themselves.

Keys and Session Tokens stay on the server, encrypted at rest. The agent's tools
only see what the company answered. Requests go through the organization's
egress allowlist.

## In the Framework

A Framework app is its own personal agent. Turn on the `poppy` feature, give
the agent `PoppyPersonalAgent`, and serve its two documents at the HTTPS address
you named:

```rust
use everruns::poppy::{PoppyKey, PoppyPersonalAgent};

let poppy = PoppyPersonalAgent::new("https://agent.example.com", PoppyKey::generate()?);
let metadata = poppy.metadata("My assistant"); // serve at /v1/poppy/agent.json
let jwks = poppy.jwks(); // serve at /v1/poppy/jwks.json
let agent = Agent::builder()
    .name("personal-agent")
    .model(model)
    .capability(poppy)
    .build()?;
```

The tools are the same. The app keeps the person's User IDs, keys and Session
Tokens in memory, so a restart is a new person to every company. See
[the example](https://github.com/everruns/everruns/tree/main/examples/poppy-personal-agent).

## Limits

- Sessions are signed out: the person does not sign in to the company yet, so a
  company that needs an account says so, and the agent tells the person.
- Company APIs and operations are not used; only the conversation.
