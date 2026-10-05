---
type: Specification
title: "Modal Sandboxes"
description: "Modal VM and gVisor sandboxes as the first module of the everruns-integrations crate: gRPC transport, credential shape, session state and leases, experimental gating, and how it is tested."
tags:
  - everruns
  - integrations
  - sandboxes
---
# Modal Sandboxes

## Why

[Modal VM sandboxes](https://modal.com/blog/vm-sandboxes-agent-computers) give
an agent a full Linux machine with its own kernel, so Docker, FUSE and system
services work where container sandboxes stop. Modal also keeps the lighter
gVisor runtime. The `modal` capability exposes both, defaulting to the VM.

## Where it lives: the `everruns-integrations` crate

Modal is the first module of `crates/integrations` (`everruns-integrations`).
The crate folds vendor integrations into one package the way `everruns-drivers`
folds LLM drivers: one Cargo feature per vendor (`modal`), every dependency
optional behind it, so a default build compiles nothing. Existing
`integrations/<name>/` crates move in over time. Rules the crate keeps:

- **Contracts only.** It depends on `everruns-contracts` (with the `runtime`
  SPI), never on `everruns-core`; `scripts/lib/check-provider-isolation.sh`
  covers it like the `integrations/` crates.
- **Same registration as other integrations.** Each module exports
  `CAPABILITY_PLUGINS` and `CONNECTOR_PLUGINS`, named in
  `crates/integrations-catalog` with crate name `everruns-integrations::<module>`.
- **Experimental first.** Modal's plugins are `experimental_only` with no
  feature flag: registered at development grade only, like Sprites, until it
  has run in production.

## Transport

Modal has no REST API. The client speaks gRPC to two services:

| Service | Endpoint | Used for |
|---|---|---|
| `modal.client.ModalClient` | `https://api.modal.com` | Auth token, app, image build, sandbox create/wait/terminate, tunnels, snapshots, router access |
| `modal.task_command_router.TaskCommandRouter` | Per-task URL returned by `TaskGetCommandRouterAccess` | Exec start, stdin, stdout/stderr streaming, exit wait |

- **Trimmed protos.** `crates/integrations/proto/modal/` holds the subset of
  modal-client's Apache-2.0 protos the client calls, with field numbers copied
  verbatim. Add fields by copying them from upstream; never renumber. `build.rs`
  compiles them with `protox` (no `protoc`) only when the `modal` feature is on.
- **Auth.** Control-plane calls carry the token pair headers plus a short-lived
  auth token from `AuthTokenGet`. Router calls carry the router JWT. Both are
  refreshed once on `Unauthenticated`; the router JWT is cached per task, in
  memory only. The router URL must be HTTPS (loopback HTTP is allowed for the
  mock server in tests).
- **Proxies.** tonic ignores `HTTPS_PROXY`, so `transport.rs` opens an HTTP
  CONNECT tunnel itself (hyper-util `Tunnel`), honouring `NO_PROXY`. Without it
  the integration cannot run behind an egress proxy, including the agent sandbox.
- **Exec.** Commands run as argv (`sh -c` for `modal_exec`). Stdin is written in
  1 MiB chunks and always closed; stdout and stderr are read concurrently and
  resumed by offset after a dropped stream; output is capped at 8 MiB per
  stream. A signal kill reports `128 + signal`, like a shell.
- **Sandboxes** run `sleep infinity` as their entrypoint and are bounded by
  Modal's own `timeout` (default 1 hour, max 24 hours).

## Credentials

The connector stores one string, the token pair `ak-...:as-...`. The server
persists only the `api_key` field of a connection, and leased-resource cleanup
receives only that token, so a two-field form would lose the secret.
`ModalCredentials::parse` also accepts the `modal token set --token-id ...
--token-secret ...` command and a whitespace-separated pair. `validate` parses
first (no network for malformed input), then calls `AuthTokenGet`.

## State and cleanup

- Sandbox state is the session secret `modal_sandbox:<sb-id>`. Core reserves
  the prefix (`INTERNAL_SECRET_PREFIXES`), so agents cannot read or forge it
  through `secret_store`; a test in the module pins the two together.
- Each sandbox is a leased resource (provider `modal`, type `sandbox`, 30-minute
  lease refreshed on use). Tools check ownership against the session's leases
  before calling Modal.
- The worker's lease cleanup (`crates/worker/src/leased_resource_cleanup.rs`)
  terminates expired sandboxes and treats Modal's "not found" as already gone.
- Every Everruns sandbox lives in the Modal app `everruns-sandboxes` and is
  tagged with the session, so a workspace owner can find them in Modal's UI.

## Testing

| Layer | Where | Runs |
|---|---|---|
| Unit | `src/modal/*` | `cargo test -p everruns-integrations --features modal,test-util` in CI |
| Tool flows against a gRPC mock | `tests/modal_tools.rs`: in-process tonic server for both services, in-memory filesystem, stale first router JWT so every run exercises refresh | Same CI job |
| Live | `tests/modal_live.rs`, feature `modal-live-tests`, `MODAL_TOKEN_ID` / `MODAL_TOKEN_SECRET` from Doppler, fail-closed when missing | `.github/workflows/modal-integration.yml` on push to `main` touching the crate, plus the weekly sweep |

The live tests boot real VM and gVisor sandboxes, check the VM runs its own
kernel, stream stdin/stdout/stderr, kill on timeout, snapshot and restore,
open a tunnel, and terminate through a guard even when an assertion fails.

## Gaps

- No Modal Volumes, Secrets or custom networking (block/allow lists) yet.
- No provider-neutral Environment target: the model sees `modal_*` tools.
- Threats: [TM-MODAL](../security/threat-model.md#18a-modal-sandbox-tm-modal).
