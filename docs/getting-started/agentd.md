---
title: Run agents with agentd
description: Run agentd on its own as the backend for your app's agents, with one binary, Docker or Kubernetes. Push agents and model keys, create sessions with labels, and read one stream of everything that happened.
sidebar:
  label: "agentd on its own"
---

agentd is the part of Everruns that runs agents' sessions. The Platform runs
it behind its API, but agentd also runs on its own, as a backend for your own
app, the way you would run a database: your app pushes agents and model keys
in, creates sessions, sends messages, and reads one stream of what happened.
agentd keeps the sessions and runs the turns. It has no users, organizations,
UI or billing of its own: your app decides who may do what, and agentd
enforces the limits your app sets.

This page starts an install, runs a turn and reads it back, then covers the
production setup. What agentd promises about your data, and how to restore
it, is in [Data plane (agentd)](/sre/agentd/). Every route is in the
[agentd API reference](/reference/agentd-api/).

## Start an install

`agentd serve` runs a whole install in one process: the shard that keeps
sessions, the executor that runs turns, and the API, on port 7700.

**Docker** is the quickest start. Make an API key, then start the image:

```bash
mkdir -p keys && printf 'app:%s\n' "$(openssl rand -hex 24)" > keys/api-keys
curl -fsSLO https://raw.githubusercontent.com/everruns/everruns/main/infra/agentd/compose.yaml
docker compose up -d
export AGENTD_KEY="$(cut -d: -f2 keys/api-keys)"
curl -fsS http://localhost:7700/health
```

A key is `id:secret`, one per line; the secret is at least 24 characters, and
callers send it as `Authorization: Bearer <secret>`.

**One binary**: each release has `agentd` for Linux (x86_64 and arm64) among
its assets. For development, `--dev` serves without keys, on localhost only:

```bash
agentd serve --dev --data ./agentd-data
```

**Kubernetes**: the `agentd` chart runs the same install as a StatefulSet
with a volume:

```bash
kubectl create secret generic agentd-keys --from-file=api-keys=keys/api-keys
helm install agentd oci://ghcr.io/everruns/charts/agentd --set apiKeys.existingSecret=agentd-keys
```

The examples below use `curl` with the key; with `--dev`, leave out the
`Authorization` header.

## Push a model key and an agent

A secret holds a value; binding it to a model provider makes it the key that
provider is called with. Here an OpenAI key, used by every agent that names
provider `openai`:

```bash
curl -fsS -X PUT http://localhost:7700/v1/secrets/openai-key \
  -H "Authorization: Bearer $AGENTD_KEY" -H 'content-type: application/json' \
  -d "{\"value\": \"$OPENAI_API_KEY\", \"bind\": {\"model\": \"openai\"}}"
```

Agents are [agent definitions](/reference/agent-package/), pushed by id:

```bash
curl -fsS -X PUT http://localhost:7700/v1/agents/helper \
  -H "Authorization: Bearer $AGENTD_KEY" -H 'content-type: application/json' \
  -d '{"name": "helper", "instructions": "Answer briefly.", "model": {"provider": "openai", "model": "gpt-6.2-sol"}}'
```

The answer carries the agent's `revision`. Pushing the same definition again
changes nothing; a different one makes the next revision. A definition agentd
cannot build is refused at push (`400`), not at the first turn.

To keep agents in git, put one `.json` or `.yaml` file per agent in a folder
(the file name is the id) and push the folder:

```bash
agentd push ./agents --url http://localhost:7700 --api-key "$AGENTD_KEY"
```

`--prune` also deletes the install's agents that have no file. Sessions
already running keep the revision they started with.

## Run a session

Create a session of an agent, with labels: any names and values your app
uses to find sessions again.

```bash
curl -fsS -X POST http://localhost:7700/v1/sessions \
  -H "Authorization: Bearer $AGENTD_KEY" -H 'content-type: application/json' \
  -d '{"agent_id": "helper", "labels": {"customer": "c-42", "channel": "web"}}'
```

Send a message. `wait=true` answers when the turn ends; without it the call
answers at once and the turn runs on.

```bash
curl -fsS -X POST "http://localhost:7700/v1/sessions/$SESSION/messages?wait=true" \
  -H "Authorization: Bearer $AGENTD_KEY" -H 'content-type: application/json' \
  -d '{"text": "What is agentd?"}'
```

Labels are kept for the session's life and ride along everywhere it shows
up. List sessions by label with `GET /v1/sessions?label=customer=c-42`.
agentd adds one label itself, `agentd.agent`, naming the pushed agent a
session was created from; label names starting with `agentd.` are agentd's.

## Read the stream

`GET /v1/stream` is everything that happened in the install, in order: every
session event, each turn's model usage and cost, session summaries, and
changes to agents and settings. Each record carries its session's labels.

```bash
curl -fsS "http://localhost:7700/v1/stream?label=customer=c-42&wait=20" \
  -H "Authorization: Bearer $AGENTD_KEY"
```

A page is `{"data": [...], "cursor": "..."}`. Pass the cursor back as
`after=` to read on from it; each record comes exactly once. `wait=` holds
the call open up to that many seconds until something arrives. Filter by
`label=name=value` (repeat it to require several) and by `type=`, either a
record's kind (`Usage`, `Session`, `Event`) or a session event's type
(`turn.completed`). With `Accept: text/event-stream` the same records come
as server-sent events, each with its cursor as the event id, so a client
that reconnects with `Last-Event-ID` picks up where it left off.

The stream keeps 30 days. A cursor older than that is refused with `410
Gone`: your app has missed records and should start again without a cursor.

## Group sessions for limits and keys

Start an install with a group label (`--group-label org`, or
`AGENTD_GROUP_LABEL`) and every session must carry it: the value is the
session's group, such as your customer or tenant. A group has:

- its own limits, set with `PUT /v1/groups/{group}`: `limit` (turns at once),
  `suspended` (no new turns start), and `spend_usd` (the most it may spend on
  models, ever). A turn that would go over the spend limit ends before it
  calls the model.
- its own data key: its secrets are sealed with it.
- its own secrets (`PUT /v1/groups/{group}/secrets/{name}`), which its
  sessions find before the install's.

Without a group label, every session is in one group, the install.

## Model keys

A turn calls its provider with the narrowest key bound to it: the session's,
then its agent's, then its group's, then the install's.

| Scope | Route |
| --- | --- |
| Install | `PUT /v1/secrets/{name}` |
| Group | `PUT /v1/groups/{group}/secrets/{name}` |
| Agent | `PUT /v1/agents/{agent}/secrets/{name}` |
| Session | `PUT /v1/sessions/{session}/secrets/{name}` |

A new or changed key applies to turns within 30 seconds.

A secret's value can come from three places:

- `value`: stored, sealed with the group's data key. Only executors open it.
- `env`: the name of an environment variable the executor reads. Off unless
  the install starts with `--env-secrets` (`AGENTD_ENV_SECRETS=true`); then
  agentd also pushes the provider keys it finds in its environment
  (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `OPENROUTER_API_KEY`,
  `GEMINI_API_KEY`, `MISTRAL_API_KEY`) as install keys for their providers.
  Handy for one person's install; it means every session shares one key.
- `resolve`: `{"ref": "..."}`, a reference your own service resolves. The
  executor sends `POST <resolver>/resolve` with `{"name", "ref", "group"}`
  and uses the `value` in the answer for `expires_in` seconds. Set the
  resolver with `AGENTD_RESOLVER_URL` (and `AGENTD_RESOLVER_TOKEN`, sent as
  a bearer token). Keys never rest in agentd at all.

Secret values never leave agentd: `GET /v1/secrets` lists names and
revisions.

## Production setup

**Credentials.** Without `--dev`, an install refuses to start until it has
a way to authenticate callers:

- API keys from `--api-keys-file` or `AGENTD_API_KEYS` (comma-separated).
- Tokens signed by your identity provider: `AGENTD_JWKS_URL`.
- Client certificates: serve TLS with `--tls-cert` and `--tls-key`, and add
  `--client-ca`; a certificate that CA signed is the credential.

`/health` answers without credentials. The first caller that pushes becomes
the install's controller; pushes from another caller are refused, so two
apps cannot fight over one install's agents.

**TLS.** `--tls-cert`/`--tls-key` (PEM) serve HTTPS on the same port.

**Storage.** `--data` holds the shard's log and snapshots and, by default,
the object store with history, restore points and the stream. On one disk,
losing the disk loses them. For anything you cannot lose, give the install an
object store (`AGENTD_STORE=s3://bucket/prefix`) and keep the data plane key
outside the volume (`AGENTD_DP_KEY`, base64 of 32 bytes): it seals every
pushed secret, and losing it loses them. [Data plane (agentd)](/sre/agentd/)
covers what each setup survives and how to restore.

**Capacity.** `--slots` (turns at once, default 64) and
`--memory-budget-mb` (memory sessions may use before idle ones are unloaded;
keep the container's limit about 1.5 times it).

`agentd serve` is one node. To keep running through a lost machine, run the
roles apart, as the Platform does: three shard nodes, executors and a gateway
with `--install` (see the [Kubernetes guide](/getting-started/kubernetes/#choose-the-data-plane)).
