---
title: Run a custom agent worker
description: Build a custom agent with private tools, connect it securely to your self-hosted control plane, and run an end-to-end smoke test.
---

Custom workers are an experimental, operator-provisioned self-hosted option.
Your process or container owns the agent implementation, model client and tools.
Everruns owns user authentication, session intake, scheduling and canonical event
storage. People keep using the usual chat UI, session API, work reports and SSE.

Start with the [custom data-agent walkthrough](/framework/examples/custom-data-agent/).
It uses the Everruns Framework, a real OpenAI model and a private SQLite database.
You can replace its runtime with another framework or entirely custom code.
Customer image hosting in Everruns Cloud is not enabled.

## Prerequisites

- A self-hosted CP and built-in worker built from the same repository revision
  as the custom worker. The worker protocol is experimental; pin the revision.
- Installation-operator access to the CP database for provisioning and revocation.
- An application-owned model credential. Workers cannot fetch CP provider keys.
- A network path from your worker to the restricted listener. Keep the trusted
  fleet listener private.

The [source example](https://github.com/everruns/everruns/tree/main/examples/custom-data-agent)
contains the executable, Dockerfiles and a repeatable real-model smoke runner.

## 1. Create the platform identity

Create an ordinary agent through the UI or API. This agent supplies its platform
identity and session binding; your binary supplies the actual execution logic.
For a local development stack with `AUTH_MODE=none`:

```bash
export EVERRUNS_API_URL=http://127.0.0.1:28701/api
curl -fsS "$EVERRUNS_API_URL/v1/agents" \
  -H 'Content-Type: application/json' \
  -d '{"name":"private-sales-agent","system_prompt":"Implementation supplied by our custom worker."}'
```

With authentication enabled, also supply your normal API bearer credential and
organization selector. Record the returned `agent_...` id. Do not give the
custom worker that API credential.

## 2. Provision a deployment credential

Drain execution for this agent before binding it. Generate a high-entropy token
into a private file; store only its digest in the CP database:

```bash
mkdir -p work
python3 - <<'PY'
import os, secrets
fd = os.open('work/custom-worker.token', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, 'w') as stream:
    stream.write(secrets.token_urlsafe(32))
PY
TOKEN_SHA256=$(sha256sum work/custom-worker.token | cut -d ' ' -f 1)
psql "$DATABASE_URL" --set=agent_id=agent_REPLACE_ME --set=digest="$TOKEN_SHA256" <<'SQL'
INSERT INTO custom_workers (org_id, agent_id, name, token_hash)
SELECT org_id, id, 'private-sales-agent', decode(:'digest', 'hex')
FROM agents WHERE public_id = :'agent_id'
RETURNING id, org_id, agent_id;
SQL
```

Require exactly one inserted row. Save its deployment UUID for rotation or
revocation. The database enforces agent/organization ownership. Replicas share
the deployment credential but use independent holder identities.

Operator SQL runs beside the CP. **The custom worker never receives
`DATABASE_URL`, `WORKER_GRPC_AUTH_TOKEN` or the CP encryption key.** Its database
is an entirely separate application resource.

## 3. Expose the restricted listener

For same-host development, restart the CP with:

```bash
CUSTOM_WORKER_GRPC_BIND_ADDR=127.0.0.1:28704
```

The listener is disabled when this variable is absent. It works with either
remote or in-process built-in workers. It serves only the restricted custom
service; the trusted `WorkerService` is not mounted there.

Non-loopback binds require TLS. On the **CP process**, configure the existing
`WORKER_GRPC_TLS_CERT` and `WORKER_GRPC_TLS_KEY` server settings. On separate
trusted worker processes, configure their server CA trust as described in
[environment variables](/sre/environment-variables/). On the custom worker, set
`CUSTOM_WORKER_CA_CERT` to that server CA's PEM file. The certificate must match
the URL's hostname. Do not set the CP's `WORKER_GRPC_TLS_CA_CERT` unless you intend
to require mutual TLS; the data-agent example does not configure a client identity.

Alternatively, keep the backend on loopback and terminate HTTPS at your ingress.
For example, a separate Caddy listener can forward only the custom protocol:

```text
https://workers.example.com:9002 {
  tls /run/tls/server.crt /run/tls/server.key
  reverse_proxy h2c://127.0.0.1:28704
}
```

Restrict network access to this listener. Never proxy the trusted fleet service
onto the customer-facing endpoint. Keep TLS verification enabled.

## 4. Start the data agent

Create the private database from the example README, then run from the repository
root with your model key supplied by your own secret manager:

```bash
export CUSTOM_WORKER_URL=http://127.0.0.1:28704
export CUSTOM_WORKER_TOKEN="$(cat work/custom-worker.token)"
export DATA_AGENT_DATABASE="$PWD/work/sales.sqlite"
cargo run -p everruns-custom-data-agent
```

The example opens its database read-only. Its `sales_summary` tool uses a bound
region parameter and exposes a single aggregate query rather than arbitrary SQL.
Its model transport accepts only `https://api.openai.com/v1/`, refuses redirects,
and preserves deployment proxy and CA configuration. It does not accept an
arbitrary model endpoint. The shared platform HTTP guards remain unchanged.

Open Playground, select the provisioned agent and ask for north-region sales. The
normal chat, events, work and trace views receive the tool call and final answer.
Tool results in events are visible to that session's readers: expose only data
they are allowed to see.

## 5. Run the full smoke test

The runner uses a **real model**, creates an agent/session through the normal
API, provisions its own isolated binding, seeds a private database and starts the
actual worker executable. Run it against a disposable local stack with a live
built-in worker and the custom listener enabled:

```bash
cargo build -p everruns-server -p everruns-worker -p everruns-custom-data-agent
# Start the normal local stack with CUSTOM_WORKER_GRPC_BIND_ADDR set on its CP.
# Supply DATABASE_URL and OPENAI_API_KEY from your operator/application secret managers.
python3 examples/custom-data-agent/smoke.py \
  --api http://127.0.0.1:28701 \
  --worker-url http://127.0.0.1:28704
```

For authenticated stacks set `EVERRUNS_TOKEN` and, if needed, pass `--org org_...`.
The API credential stays in the smoke runner; it is not passed to the worker.
`--image IMAGE --ca /path/to/ca.pem` runs the same test with a Docker worker
instead of a process. Use a worker URL reachable from that container. The
[example README](https://github.com/everruns/everruns/tree/main/examples/custom-data-agent#full-smoke-test)
explains container builds and CA mounts.

The runner checks:

- Queued input is not claimed by the built-in worker while custom code is absent.
- The real model calls the private tool and reports the seeded aggregate.
- Canonical generations, tool events, live SSE output, messages, work reports,
  trace correlation and the final idle status reach the CP.
- A fresh worker replica resumes from CP conversation history and sees a private
  database update.
- Revocation rejects the credential without falling back to built-in execution.
- Rotation completes the original queued input without a duplicate terminal event.

It writes a JSON report and event evidence under `work/`, stops its worker and
revokes its credential even on failure. It retains its isolated agent/session for
inspection. It does not stop or delete the operator's stack. Live runs incur
model charges; an ignored model test alone is not evidence of a completed smoke.

## Implement another runtime

Use the checked-in
[protobuf contract](https://github.com/everruns/everruns/blob/main/crates/internal-protocol/proto/worker.proto)
to generate a client in your language. Authenticate every call with the deployment
bearer credential. The narrow service supports:

| Operation | Responsibility |
| --- | --- |
| `LeaseActors` | Poll for sessions assigned to this agent; use a unique replica holder. |
| `ReadJournal` | Read and page the CP log under the current holder/fence. |
| `RenewActorLease` | Renew during execution and observe cancellation or new input. |
| `EmitEvent` | Send typed canonical execution events with the live lease fence. |
| `ReleaseActor` | Acknowledge only the input that finished; leave newer input queued. |

Stop execution when renewal or a fenced append fails. The CP namespaces turn and
trace ids and stamps deployment provenance. A recorded input owns one custom
turn. Generation reports cannot claim a CP-managed provider account. Report
model generations and tool outcomes truthfully to preserve useful observability.

## Revoke, recover and isolate

Disable a credential with operator SQL:

```sql
UPDATE custom_workers SET enabled = FALSE WHERE id = 'DEPLOYMENT_UUID';
```

Subsequent requests fail authentication; an already authorized request may finish.
The routing binding remains. Pending input waits for custom code rather than
silently running the built-in implementation. To rotate, replace `token_hash`
with a new random token's digest, enable the row and update your replicas.

Deleting the binding explicitly restores built-in routing. Stop replicas, allow
leases to expire and resolve interrupted turns before deleting or rebinding;
this is not a live migration procedure.

Custom code owns its tool policy, model budget and side-effect recovery. Canonical
event reporting does not prove that arbitrary code obeyed the built-in Act,
approval or guardrail pipeline. The example replays interrupted **read-only**
queries, limits retries and conversation size, and handles cancellation. Write
tools need their own idempotency and interruption contract. Client tools,
approvals, mid-turn steering, delegation and background work are outside this
example. See its README for exact limits.

Run customer images in infrastructure isolated from the CP, without privileged
host mounts, CP credentials or access to private fleet endpoints. Authenticating
a remote executor is not a container sandbox. Cloud image hosting requires a
separate isolation design and is not enabled by this feature.
