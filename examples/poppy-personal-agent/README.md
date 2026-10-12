# Poppy personal agent

A Framework app that acts as a person's own agent toward companies, over the
[Personal Agent Protocol](https://personalagentprotocol.org) ("Poppy", draft
0.1). The person asks about an order; the agent finds the company by its
domain, messages the company's agent, and reports what the company said.

The app is the personal agent. It signs with its own ES256 key and serves its
client metadata (`/v1/poppy/agent.json`, the URL that is its `client_id`) and
its key (`/v1/poppy/jwks.json`). Companies fetch both to check the agent's
assertions, so they must be reachable over HTTPS at a public address.

It runs on OpenAI's `gpt-6.1-sol`. A run against
[pi-poppy](https://github.com/richardanaya/pi-poppy)'s company, which asks a
signed-out person to sign in before it discusses an order:

```console
person> Ask shop.poppy.localhost:46225 whether my order ord_7Hk2 has shipped, and tell me what they said.
  poppy_discover {"company":"shop.poppy.localhost:46225"}
  poppy_send {"company":"shop.poppy.localhost:46225","text":"I'm acting as the customer's personal assistant. Has their order ord_7Hk2 shipped? Please provide its current shipping status."}
agent> They said: "I need you to sign in before I can see that order."
```

## Run it

You need a company that publishes `/.well-known/poppy.json`, and a public
HTTPS address that reaches this process (a reverse proxy or a tunnel to
`127.0.0.1:8787`):

```bash
OPENAI_API_KEY=... POPPY_AGENT_URL=https://agent.example.com \
  cargo run -p everruns-poppy-personal-agent -- shop.example.com
```

`POPPY_AGENT_PORT` changes the local port. A second argument replaces the
question.

## Test it

`tests/pi_poppy.rs` runs the same agent against pi-poppy's company on this
machine, with the company's checks on (client metadata, assertions, DPoP proofs
with nonces). It needs Node 22+ and openssl, and runs when `PI_POPPY_DIR` and
`OPENAI_API_KEY` are set:

```bash
doppler run -- scripts/poppy-interop.sh
```

## Limits

- State lives in memory. Each run is a new person to every company, with a new
  key.
- Sessions are signed out: a company that needs an account says so.
- A retried turn sends a new message rather than repeating the last one.
