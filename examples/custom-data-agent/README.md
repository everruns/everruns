# Custom data agent worker

An experimental customer-hosted worker built with the Everruns Framework. It
connects outbound to the control plane, leases sessions for one provisioned
agent, runs its own model/tool logic, and sends canonical events back. Users
keep using the normal Everruns UI, messages API and SSE.

The application's model transport accepts only OpenAI's HTTPS API origin,
refuses redirects, and preserves deployment proxy/CA configuration. It does not
accept arbitrary model base URLs. The shared platform HTTP guards are unchanged.

The worker holds its own OpenAI key and a private SQLite database. The CP
receives query results in tool events, never the database file or credentials.
This is an application deployment example, not a container sandbox.

## 1. Prepare a self-hosted control plane

Use this repository revision for the server and worker. Create an ordinary agent
through the existing UI or API; its definition supplies the platform identity.
The custom binary owns the actual instructions and tool implementations.

Stop/drain execution for that agent before binding it. Apply migrations using
the normal server startup. Operator access to the **CP database** is required
only for provisioning; never give that access to the custom worker.

Generate a random credential into a private file (without a trailing newline):

```bash
mkdir -p work
python3 - <<'PY'
import os, secrets
fd = os.open('work/custom-worker.token', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, 'w') as stream:
    stream.write(secrets.token_urlsafe(32))
PY
TOKEN_SHA256=$(sha256sum work/custom-worker.token | cut -d ' ' -f 1)
psql "$DATABASE_URL" --set=agent_id=agent_REPLACE_ME --set=token_hash="$TOKEN_SHA256" <<'SQL'
INSERT INTO custom_workers (org_id, agent_id, name, token_hash)
SELECT org_id, id, 'data-agent', decode(:'token_hash', 'hex')
FROM agents WHERE public_id = :'agent_id'
RETURNING id, org_id, agent_id, name;
SQL
```

Require exactly one inserted row. Keep the returned deployment id for rotation
and revocation. One agent has one deployment; replicas share the deployment
credential and use independent holder identities.

Enable the restricted listener on the CP:

```bash
CUSTOM_WORKER_GRPC_BIND_ADDR=127.0.0.1:9002
```

Restart the server with this variable. It works with the in-process development
worker too. For a remote listener, configure the existing `WORKER_GRPC_TLS_*`
server certificate/key settings and bind an appropriate address. Non-loopback
binds without TLS fail startup. Expose **only the custom listener**, keeping the
trusted fleet listener private. Client requests never need `WORKER_GRPC_AUTH_TOKEN`.

## 2. Create the application's private database

```bash
python3 - <<'PY'
import sqlite3
with sqlite3.connect('work/sales.sqlite') as db:
    db.executescript('''
      CREATE TABLE sales (region TEXT NOT NULL, amount_cents INTEGER NOT NULL);
      INSERT INTO sales VALUES ('north', 12000), ('north', 8500), ('south', 7000);
    ''')
PY
```

The tool opens this file read-only and uses a bound region parameter. It exposes
one aggregate query, not arbitrary model-generated SQL.

## 3. Run the worker

```bash
export CUSTOM_WORKER_URL=http://127.0.0.1:9002
export CUSTOM_WORKER_TOKEN="$(cat work/custom-worker.token)"
export DATA_AGENT_DATABASE="$PWD/work/sales.sqlite"
# Supply OPENAI_API_KEY from your own secret manager.
cargo run -p everruns-custom-data-agent
```

For HTTPS also set `CUSTOM_WORKER_CA_CERT` to the PEM CA file that issued the
server's certificate. Use a certificate whose name matches the URL. Keep TLS
verification enabled. A deployment that requires mutual TLS additionally needs
a client identity; this sample does not configure one.

Open Playground and select the provisioned agent in the normal Everruns UI. Ask:
“How many north-region orders are there, and what is their total revenue?”
The agent calls `sales_summary`, reporting two orders and 20,500 cents. Tool
calls, model usage, final answers and turn events use the CP's ordinary event
pipeline. The implementation can be changed to another framework or a custom
loop by implementing the same restricted protobuf service contract.

## Container packaging

Build from the repository root:

```bash
docker build -f examples/custom-data-agent/Dockerfile -t custom-data-agent .
docker run --rm --read-only --cap-drop=ALL --security-opt=no-new-privileges \
  --user=65532:65532 --pids-limit=128 --memory=512m --cpus=1 \
  --mount=type=bind,source="$PWD/work/sales.sqlite",target=/data/sales.sqlite,readonly \
  --env=CUSTOM_WORKER_URL --env=CUSTOM_WORKER_TOKEN --env=OPENAI_API_KEY \
  --env=DATA_AGENT_DATABASE=/data/sales.sqlite custom-data-agent
```

Use a reachable HTTPS CP URL in containers; container loopback is not the host.
Mount the server CA read-only and set `CUSTOM_WORKER_CA_CERT` for HTTPS. Restrict
network access to the custom listener and the model endpoint in your deployment.
These flags are an example of operator hardening, not a claim that arbitrary
customer images are safe to run next to the CP. No image hosting is added to
Everruns Cloud.

## Rotate or revoke

Operator SQL, with the deployment UUID from provisioning:

```sql
UPDATE custom_workers SET enabled = FALSE WHERE id = 'REPLACE_WITH_DEPLOYMENT_UUID';
```

Subsequent requests fail authentication. The binding remains, so sessions wait
for custom code instead of falling back to built-in execution. To rotate, replace
`token_hash` with a newly generated digest and enable it again, then restart
replicas with the new token. An already authorized in-flight request may finish.

Deleting the binding explicitly returns sessions to the built-in fleet. First
stop replicas, wait for live leases to expire, and finish or cancel interrupted
turns. Rebinding an active lease is not a supported migration procedure.

## Recovery and limits

The sample processes one input at a time, renews every five seconds and releases
only the completed input. New input queues for the next turn. Cancel requests
stop local Framework execution; lease loss stops forwarding and cancels local
work. Control-plane events remain the durable record. A crash replays an
interrupted **read-only** operation, with a stable turn id and a bounded retry
count. A persisted final answer can finish without repeating the query.

The example reconstructs conversation context each turn, with a 2,000-event
limit. It does not claim native Engine checkpoint recovery or exactly-once
external effects. Arbitrary write tools require their own idempotency and
recovery contract. Approvals, client tools, hosted credentials, background tasks,
steering within a turn and delegation are outside this sample. Custom code owns
its own tool policy and model budget; the CP cannot enforce the built-in Act
pipeline inside arbitrary code. All tool results forwarded as events are visible
to that session's readers, so expose only data those readers may see.

## Full smoke test

This is a real-model integration test, not a simulator. Run against a disposable
self-hosted stack with a live built-in worker and the restricted custom listener
configured. The runner creates its own agent, Playground session, deployment
credential and private SQLite database. It needs Python 3, curl, psql, the built
worker binary and operator `DATABASE_URL`; authenticated stacks also need
`EVERRUNS_TOKEN` and optionally `--org org_...`.

```bash
cargo build -p everruns-server -p everruns-worker -p everruns-custom-data-agent
# Start your normal stack with CUSTOM_WORKER_GRPC_BIND_ADDR=127.0.0.1:28704.
# Supply DATABASE_URL and OPENAI_API_KEY through your secret manager.
python3 examples/custom-data-agent/smoke.py \
  --api http://127.0.0.1:28701 \
  --worker-url http://127.0.0.1:28704
```

The API URL can be the direct CP origin, the normal reverse-proxy origin, or an
origin ending in `/api`. The runner uses the server's `/api/v1` namespace and
checks `/health` separately.

It proves queuing without built-in fallback; real model/tool execution; canonical
usage, messages, work and trace reports; live SSE; replica replacement; fresh
private data; revocation without fallback; and credential rotation completing
already queued input. Three turns must complete without duplicate terminal
events. It stops its worker and disables its credential on every exit. Agents,
sessions and private JSON evidence remain under the chosen artifact directory
for inspection. Do not run against production data. Live tests incur model costs.

### Container smoke

Build the full source image above, or package an existing Linux binary with
`Dockerfile.runtime`. The latter uses Debian 13 and requires a compatible binary;
it avoids a second Rust compilation when a CI or local build already exists:

```bash
mkdir -p work/custom-worker-bin
cp target/debug/everruns-custom-data-agent work/custom-worker-bin/
chmod 755 work/custom-worker-bin/everruns-custom-data-agent
docker build -f examples/custom-data-agent/Dockerfile.runtime \
  -t custom-data-agent:smoke work/custom-worker-bin
```

Put an HTTPS ingress in front of the loopback custom listener, as shown in the
[public how-to](https://everruns.com/docs/how-to/custom-agent-workers/). Use a
server leaf certificate signed by your test/deployment CA, with the hostname or
IP from `CUSTOM_WORKER_URL` in its Subject Alternative Name. A CA certificate
itself is not a server leaf. Keep the CA private key on the operator side; the
worker receives only the public CA certificate.

```bash
python3 examples/custom-data-agent/smoke.py \
  --api http://127.0.0.1:28701 \
  --worker-url https://WORKER_REACHABLE_HOST:9002 \
  --image custom-data-agent:smoke \
  --ca /path/to/public-ca.pem
```

The runner checks the actual container is non-root, unprivileged, uses a read-only
root and mounts, drops capabilities, enforces memory/PID limits and uses bridge
networking. It also verifies CP credential names are absent from the container's
environment. The credential env-file is private and removed during cleanup.

In environments with an HTTPS inspection proxy, preserve the configured Docker
proxy defaults and supply its CA trust. Both Dockerfiles accept an optional
BuildKit `system_ca` secret; it exists only for networked build steps. The smoke
runner's `--proxy-ca /path/to/system-ca-bundle.pem` mounts a public trust bundle
read-only in the running worker. Neither option disables TLS verification or
copies session-specific trust into an image layer.

```bash
docker build --secret id=system_ca,src=/path/to/system-ca-bundle.pem \
  -f examples/custom-data-agent/Dockerfile.runtime \
  -t custom-data-agent:smoke work/custom-worker-bin
```

### Offline and focused tests

```bash
cargo test -p everruns-custom-data-agent
cargo test -p everruns-server --lib worker_link::custom_workers
cargo test -p everruns-server --lib storage::repositories::session_actors::tests
python3 -m unittest discover -s examples/custom-data-agent/tests
```

The example's cargo tests include its Python harness contracts, so ordinary CI
also checks API namespace/health handling and credential/CA deployment boundaries.
The ignored `real_model_queries_the_private_database` test is a useful provider
preflight, but does not replace the complete CP/worker smoke above.
