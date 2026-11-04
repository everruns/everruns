# Framework Engine inside celld

This is the celld placement of the [shared Framework application](../durable-object-agent).
Its Agent module imports `everruns` and defines an ordinary `Agent` and `FunctionTool`.
The Worker imports that same application: `Engine` owns admission, lifecycle hooks,
execution and recovery. One Durable Object authority owns each tenant/session.
There is no celld-specific Reason/Act or step machine.

```sh
bash build.sh
# Configure distinct API_TOKEN and AUTHORITY_TOKEN (at least 32 bytes each),
# OPENAI_API_KEY, and optionally OPENAI_BASE_URL and MODEL in .dev.vars.
celld dev .
```

Use the standard `/v1/sessions`, `/messages`, `/events` and `/sse` endpoints.
The reference configuration uses celld dev's loopback ingress for notification
WebSockets. For a fleet, set `NOTIFICATION_ORIGIN` to a fixed trusted Worker
ingress that forwards WebSocket upgrades. The authority credential is sent there;
redirects are refused. This uses celld 0.6.2's HTTP socket path because its kept
cross-node stub sockets do not deliver notifications. Cloudflare uses its direct
stub socket path. Engine's session and execution interfaces are shared.

Retain creation and message identities across retries. A successful creation
commits the binding; a successful send commits its receipt and wakeup obligation.
Read admissions separately for assignment and completion. API credentials never
authorize the internal authority routes.

The shared [qualification harness](../../crates/durable-object/worker/test/qualify-celld.mjs)
starts three celld nodes over replicated storage, loses the owner during a tool,
and requires recovery without another client request. Its topology and storage
limits apply; local `celld dev` alone is not a replicated durability proof.

Isolate tools cannot run OS processes. For those use the
[serve native placement](../serve/celld), whose containers execute Engine against
the same per-session authority. Workspace persistence is an independent service.
