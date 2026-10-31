---
title: Run a custom agent worker
description: Build a custom agent with private tools, connect it securely to your self-hosted control plane, and run an end-to-end smoke test.
---

For a customer-hosted HTTPS endpoint, including Cloudflare Workers, see
[Connect a Cloudflare agent](/how-to/cloudflare-agent/). That opt-in transport
has API/UI registration and reuses the restricted worker event pipeline.
The pull-worker process/container flow below remains operator-provisioned.

Custom pull workers are an experimental, operator-provisioned self-hosted option.
Your process or container owns the agent implementation, model client and tools.
Everruns owns user authentication, session intake, scheduling and canonical event
storage. People keep using the usual chat UI, session API, work reports and SSE.

Start with the [custom data-agent walkthrough](/framework/examples/custom-data-agent/).
It uses the Everruns Framework, a real OpenAI model and a private SQLite database.
You can replace its runtime with another framework or entirely custom code.
Customer image hosting in Everruns Cloud is not enabled.

## Where it appears in the UI

The custom implementation belongs to an **ordinary Agent**. Create that Agent in
**Agents**, then have an installation operator bind its ID to a custom-worker
deployment. It stays in the Agents list with its usual name, avatar and
Integrations page. Its conversations appear as ordinary sessions; open
**Playground** to talk to it and **Trace** to inspect its reported execution.
It does not appear only as an anonymous collection of sessions.

Starting a worker does **not** create an Agent automatically. There is currently
no self-service pull-worker registration screen, custom-runtime badge, worker health
panel or UI editor for the pull binding. HTTPS bindings have the separate setup
flow linked above. Provisioning and credential rotation use the
operator steps below. An offline worker leaves messages queued.

The Agent's platform identity and channel settings remain useful. Its model,
system prompt, capabilities, MCP configuration and tool definitions are **not
automatically applied to your binary**. Your implementation must explicitly load
and honor any configuration it supports. The sample pins its own model,
instructions and private tool in code.

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

## Define the custom agent and tool

The worker is an application you build. This Rust building block defines its
actual behavior using the Framework (`everruns` with the `openai` feature),
`rusqlite` with `bundled`, `serde_json`, `tokio` and `anyhow`:

```rust
use std::path::{Path, PathBuf};
use everruns::{Agent, FunctionTool, Provider};
use serde_json::{Value, json};

fn sales_summary(database: &Path, region: &str) -> Result<String, String> {
    let db = rusqlite::Connection::open_with_flags(
        database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).map_err(|_| "Database unavailable".to_string())?;
    db.query_row(
        "SELECT count(*), coalesce(sum(amount_cents), 0) FROM sales WHERE region = ?1",
        [region],
        |row| Ok(json!({
            "region": region,
            "orders": row.get::<_, i64>(0)?,
            "revenue_cents": row.get::<_, i64>(1)?
        }).to_string()),
    ).map_err(|_| "Sales query failed".to_string())
}

fn build_sales_agent(database: PathBuf, provider: Provider) -> anyhow::Result<Agent> {
    let tool = FunctionTool::new(
        "sales_summary",
        "Read aggregate sales from our private database.",
        json!({
            "type": "object",
            "properties": {"region": {"type": "string"}},
            "required": ["region"],
            "additionalProperties": false
        }),
        move |args: Value| {
            let database = database.clone();
            async move {
                let region = args["region"].as_str()
                    .ok_or_else(|| "region required".to_string())?.to_owned();
                tokio::task::spawn_blocking(move || sales_summary(&database, &region))
                    .await.map_err(|_| "Database task failed".to_string())?
            }
        },
    );
    Ok(Agent::builder()
        .name("private-sales-agent")
        .provider(provider)
        .model("gpt-6.1-sol")
        .instructions("Use sales_summary for sales questions. Never invent numbers.")
        .max_iterations(8)
        .tool(tool)
        .build()?)
}
```

Supply an application-owned provider, for example
`everruns::OpenAI::from_env()?.into()` from `OPENAI_API_KEY`, when calling
`build_sales_agent(database, provider)`. The checked-in worker uses its
fixed-origin transport to preserve deployment proxies and CA trust, and also
reconstructs prior conversation in its instructions.

This function builds the agent; it does **not** register it with the CP or replace
the worker protocol loop. The adapter in
[`src/main.rs`](https://github.com/everruns/everruns/blob/main/examples/custom-data-agent/src/main.rs)
leases a session, reads its journal, runs this kind of Framework agent, forwards
canonical events, renews the lease and acknowledges the completed input. Copy
that adapter and replace its `build_agent`/tool logic to build your own worker.
Running `Engine::new().create(agent)` alone creates a local Framework session,
not a CP-connected worker.

## 4. Start the data agent

Create a private application database. This seed makes north-region sales total
27,278 cents across two orders:

```bash
python3 - <<'PYTHON'
import sqlite3
from pathlib import Path
path = Path("work/sales.sqlite")
path.parent.mkdir(parents=True, exist_ok=True)
with sqlite3.connect(path) as db:
    db.execute("CREATE TABLE sales(region TEXT NOT NULL, amount_cents INTEGER NOT NULL)")
    db.executemany("INSERT INTO sales VALUES (?, ?)", [
        ("north", 18037), ("north", 9241), ("south", 7000)
    ])
PYTHON
```

Run from the repository root with `OPENAI_API_KEY` supplied by your own secret
manager:

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

## Invoke it through the CP

The worker polls for work; users never call its container directly. In the UI,
open the Agent in Playground and send a message. For API callers, use the same
Agent ID you bound above. This local-development example uses `jq` and the API
base from step 1:

```bash
export AGENT_ID=agent_REPLACE_ME
SESSION_ID=$(curl -fsS "$EVERRUNS_API_URL/v1/sessions" \
  -H 'Content-Type: application/json' \
  -d "{\"agent_id\":\"$AGENT_ID\",\"source\":\"playground\"}" | jq -r .id)

curl -fsS "$EVERRUNS_API_URL/v1/sessions/$SESSION_ID/messages" \
  -H 'Content-Type: application/json' \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"Use sales_summary to report north-region orders and revenue_cents."}]}}'

# Read responses after the worker finishes, or follow the live SSE endpoint.
curl -fsS "$EVERRUNS_API_URL/v1/sessions/$SESSION_ID/messages"
curl -N "$EVERRUNS_API_URL/v1/sessions/$SESSION_ID/sse?after_sequence=0"
```

With authentication enabled, add your normal API bearer credential and
organization selector to these requests. Message submission is asynchronous: the
first response acknowledges input, not a completed model answer. The tool should
return `{"region":"north","orders":2,"revenue_cents":27278}`; the model composes
the final reply. Open `/playground/<session_id>` or
`/sessions/<session_id>/trace` in the platform UI to inspect this same session.
Send follow-ups to the same session's messages endpoint to retain conversation
history. Stop the SSE client with Ctrl+C when finished.

## Connect the same Agent to Slack

Yes: open **Agents → your Agent → Integrations → Add channel → Slack** and follow
[Publish an Agent to Slack](/how-to/publish-to-slack/). Keep the Agent's
**Communication** setting at **Direct** for this sample. The CP needs its normal
Slack workspace installation, public HTTPS webhook origin and channel
permissions; registering a custom worker does not perform that setup.

The basic route is:

```text
Slack message → CP channel/session intake → this Agent's actor queue
              → your worker → canonical output events → CP delivery → Slack
```

The channel must target the **same bound Agent**. Slack credentials stay in the
CP. The worker emits completed agent messages and turn lifecycle events; it does
not need the Slack bot token to return a normal reply. Channel response policy
and thread/channel/user session routing remain CP-owned. If the custom worker is
offline or revoked, accepted Slack input stays queued too.

This is the existing channel delivery path, not a separate Slack implementation
inside the worker. The shipped smoke qualifies API, Playground, Trace and SSE;
**a live Slack workspace end-to-end run with this custom worker has not been
qualified**. Verify a mention and threaded follow-up in your workspace before
relying on that deployment.

Do not infer full native Slack feature support from basic replies. The sample
has no CP `send_message` tool, Slack action tools, approval cards or delegated-task
progress implementation. **Explicit** communication therefore does not work as
an automatic replacement for Direct replies. Adding a Slack capability or MCP
server in the Agent editor does not install its tools into this worker. Your
private-data authorization must also account for channel participants; the
sample's region filter is a query parameter, not a per-Slack-user access policy.

## Functionality and ownership

| Functionality | Custom-worker behavior |
| --- | --- |
| Agents list, branding and Integrations | Ordinary Agent record; binding is operator-managed. |
| API messages and Playground | Normal CP intake, routed to the bound implementation. |
| SSE, Trace, work reports and reported usage | Existing CP views consume the canonical events your worker emits. |
| Slack basic request/reply | Existing CP channel route with Direct communication; live custom-worker Slack qualification is still required. |
| Conversation history and replica replacement | Sample reconstructs history from CP events between turns. |
| Cancellation and interrupted reads | Sample observes cancellation, renews leases and bounds read-only retries. |
| Agent editor model, prompt, capabilities and MCP tools | Not inherited by the custom implementation. |
| Tool permissions, approvals, guardrails and model budgets | Implementation must enforce its own policy; native pipeline guarantees do not transfer. |
| Arbitrary database writes and other side effects | Implementation must provide idempotency and recovery; sample tool is read-only. |
| Images, files and multimodal input | Sample extracts text only; custom implementations must add attachment handling and authorization. |
| Client tools, mid-turn steering, delegation and background work | Outside the sample's supported contract. |
| Hosting customer images in Everruns Cloud | Not enabled. |

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
model charges; compilation or unit tests alone are not evidence of a completed smoke.

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
