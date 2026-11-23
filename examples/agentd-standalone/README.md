# agentd on its own: a controller

agentd as the backend of your own app. `controller.py` is the app: it owns
customers, keys and limits, and uses agentd to keep sessions and run turns.

It pushes an OpenAI key and the agents in `agents/`, gives each customer
(a group) a spend limit, runs a short conversation per customer through two
agents (`triage` labels the question, `support` answers it), then reads the
stream for the cost of each customer's turns.

## Run it

Start an install whose groups are customers. With the `agentd` binary from a
release, for development:

```bash
agentd serve --dev --group-label customer --data ./agentd-data
```

Or from this repository: `cargo run -p everruns-agentd -- serve --dev --group-label customer`.

Then, standard library Python only:

```bash
export OPENAI_API_KEY=...
python3 examples/agentd-standalone/controller.py
```

Against an install with API keys, set `AGENTD_URL` and `AGENTD_API_KEY`.
Docker: `AGENTD_GROUP_LABEL=customer docker compose -f infra/agentd/compose.yaml up -d`
(see the compose file for the key file it reads).

Expected output: each agent's revision, then per customer the question, its
topic and the answer, then each customer's cost:

```text
agent support: revision 1
agent triage: revision 1

[acme] (repair) My rear brake squeaks. How long would a repair take?
  -> Repairs take two working days. ...

[globex] (opening-hours) Are you open on Mondays?
  -> We're closed on Mondays; we're open Tuesday to Saturday, 10:00 to 18:00.

acme: $0.0007 this run, $0.0007 of $0.50 spent
globex: $0.0006 this run, $0.0006 of $0.50 spent
```

Running it again pushes the same agents (their revisions stay) and adds to
each customer's spend.

## What it shows

- Model keys pushed as secrets and bound to a provider; agents name only the
  provider. `PUT /v1/agents/{agent}/secrets/{name}` or
  `/v1/sessions/{session}/secrets/{name}` would give one agent or one session
  its own key.
- Groups (`--group-label customer`): each customer's spend limit, checked
  before every model call.
- Labels: `topic` is set from the triage agent's answer, and every stream
  record about the session carries it.
- The stream (`GET /v1/stream?type=Usage`): one ordered feed of everything,
  read with a cursor.

The full guide is [Run agents with agentd](https://everruns.com/docs/getting-started/agentd/).
