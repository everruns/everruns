# Drive Everruns over the REST API

Everruns is API-first: everything the UI does is available over HTTP, with
server-sent events for streaming. Use the person's own Everruns unless they
name another one.

## Base URL

Set `EVERRUNS_URL` to the deployment's origin and use `$EVERRUNS_URL/api`:

- On their machine (`everruns docker up`, `everruns k8s port-forward`):
  `http://localhost:9300`
- On their server or cluster: the public URL they chose, such as
  `https://everruns.example.com`
- The demo instance, only when the person asks for it: `https://app.everruns.com`.
  It is a demo, not a hosted service.

Check it first: `curl -fsS "$EVERRUNS_URL/health"`. The deployment's own
`$EVERRUNS_URL/auth.md` says which credentials it needs; a local install with
`--auth none` needs none.

Management endpoints use `$EVERRUNS_URL/api/v1/platform/`. Use that prefix for
new integrations. Legacy management URLs under `/api/v1/` remain supported;
their responses announce deprecation on December 22, 2026 and link to the
canonical URL. No automatic removal is scheduled. Published Agent Execution
API, AG-UI and A2A URLs keep their existing addresses; use the URL returned by
the deployment rather than adding the management prefix to it.

## Credential

The person issues a personal access token in the UI under **Settings →
Personal access tokens** and puts it in their environment as
`EVERRUNS_TOKEN`. Tokens start with `evr_pat_` and are limited by their granted
scopes and the account's permissions. Never ask for the token in the
conversation, print it, or commit it. Send it as `Authorization: Bearer $EVERRUNS_TOKEN`.

## Create an agent

```bash
curl -fsS -X POST "$EVERRUNS_URL/api/v1/platform/agents" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"assistant","display_name":"Assistant","system_prompt":"You are helpful."}'
```

## Open a session

```bash
curl -fsS -X POST "$EVERRUNS_URL/api/v1/platform/sessions" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"agent_id":"<agent_id>"}'
```

## Send a message

```bash
curl -fsS -X POST "$EVERRUNS_URL/api/v1/platform/sessions/<session_id>/messages" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"Hello!"}]}}'
```

## Stream events

```bash
curl -N "$EVERRUNS_URL/api/v1/platform/sessions/<session_id>/sse" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN"
```

Sessions are durable: progress is persisted per step, so a session survives
restarts and worker loss. Disconnect, reconnect later, and resume the stream
instead of replaying from scratch.

## Reference

- OpenAPI description: <https://everruns.com/docs/api/>, or
  `$EVERRUNS_URL/.well-known/api-catalog` on the deployment
- Streaming: <https://everruns.com/docs/how-to/stream-events/>
