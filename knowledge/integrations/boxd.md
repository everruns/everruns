---
type: Specification
title: "boxd Sandboxes"
description: "boxd.sh KVM machines as a managed Sandbox Template target and as boxd_* agent tools: regions, gRPC client and login, lifecycle mapping, egress, tests, and what is left out."
tags:
  - everruns
  - integrations
  - sandboxes
---
# boxd Sandboxes

## Why

[boxd](https://boxd.sh/) runs persistent KVM machines (Ubuntu 24.04, own
kernel, Docker preinstalled, 100 GiB disk). A machine lives until deleted, and
hibernating it keeps memory and running processes while billing only disk.
For agents that work in turns with long idle gaps, that is the shape no other
provider offers: the dev server, the Docker container and the half-built cache
are all still there on the next turn, and the idle time costs almost nothing.

## How it compares

| | Daytona | E2B | Modal | boxd |
|---|---|---|---|---|
| Isolation | Container | Firecracker microVM | gVisor or VM | KVM microVM; `isolated` also strips boxd's CLI and credentials from the guest |
| Lifetime | Until deleted | Max timeout | Max 24 h | Until deleted |
| Pause keeps processes | No | Yes | No | Yes (hibernate: disk-only billing) |
| Everruns durability | `checkpointed` | `provider_snapshot` | `provider_snapshot` | `provider_snapshot` |
| Regions exposed | No | No | No | `fra`, `nyc` |
| Egress enforced outside the guest | Yes | Yes | Yes | Yes, DNS included |
| Through the Everruns egress proxy | Yes | Yes | No | No (boxd has no outbound proxy hook) |

## Where it lives

A `boxd` module in `everruns-integrations` behind the `boxd` Cargo feature,
listed in `everruns-capabilities`' hosted integration catalog. No new crate.
The gRPC client is generated from boxd's published proto, vendored verbatim in
`crates/integrations/proto/boxd/api.proto`. The gRPC transport is shared with
Modal (`grpc_transport.rs`).

Two surfaces, both opt-in by configuration, so no feature flag:

- **Sandbox Template target** `managed` / provider `boxd`
  (`session_sandbox.rs`). Options: `region` (`fra`, `nyc`), `size` (`1x4`,
  `2x8`, `4x16`, `8x32`, boxd's vCPU x GiB notation), `snapshot` (boot from a
  boxd snapshot; excludes `size`, since a snapshot keeps its own),
  `workspace_path` (default `/home/boxd/workspace`) and `title`. The server
  validates them with the provider's own parser (`boxd::validate_options`), so
  a saved template never holds options creation would refuse.
- **`boxd` capability**: `boxd_create_sandbox`, `boxd_exec`,
  `boxd_read_file`, `boxd_write_file`, `boxd_list_sandboxes`,
  `boxd_manage_sandbox` (status, hibernate, wake, delete), with the same
  session-secret and lease pattern as the E2B and Modal tools.

The connection is an API key (`bxd_...`), validated by exchanging it and
calling `Whoami`. User, agent and organization credential sources work like
the other providers.

## Client decisions

- **Login.** The key is exchanged at `app.boxd.sh/api/v1/auth/token` for a
  one-hour JWT. JWTs are cached process-wide per key and renewed five minutes
  early; a call answered `UNAUTHENTICATED` renews once and retries.
- **Regions.** Each region has its own endpoint, from `ListRegions` (cached
  ten minutes). No region asked for means the org's default
  (`GetOrgMachineDefaults`), else the home region. A region the org has not
  enabled is refused with boxd's state for it.
- **Ids carry the region.** A machine's external id is `<region>/<id>`, so
  the orphan scan, the cleanup worker and every tool call go straight to the
  right endpoint with nothing but the id and the key.
- **boxd's idle timers are off.** boxd suspends and hibernates on network
  silence, which would put a long CPU-only build to sleep mid-turn. Everruns'
  own idle policy decides when a machine hibernates.
- **No server-side delete timer on managed machines.** boxd's `delete_after`
  counts from creation, while Everruns deletes paused sandboxes on its own
  retention schedule; a timer would delete a machine a session still owns.
  The live tests set one as a backstop.

## Lifecycle

| Everruns | boxd |
|---|---|
| create | `CreateVm` (or `CreateVmFromSnapshot`) with `isolated`, size, labels, egress allowlist in the spec; wait for `running`; make the workspace |
| pause | `HibernateVm` |
| resume | `WakeVm`, or `StartVm` for a machine boxd stopped; re-apply egress when the policy changed |
| resume of a failed or missing machine | create a replacement, destroy the old one, release its lease |
| delete | `DestroyVm` (already gone counts as deleted) |
| exec | streaming `Exec`: stdout and stderr apart, 8 MiB per stream, timeout cancels the call (boxd kills the process group) and reports exit code 124 |
| read / write file | `FsRead` / `FsWrite` (parents created) |
| status | `GetVm`: hibernated, suspended and stopped are paused, failed is lost |
| orphan scan | `ListVms` per enabled region with the `everruns.*` labels, re-checked client-side |

## Egress

The template network policy and the session access list map one to one onto
boxd's allowlist: open is `[]`, blocked is `["none"]`, an allowlist is its
domains and IPv4 ranges. boxd refuses IPv6 (machines have no IPv6 egress) and
caps the list at 256 entries; both fail template validation and machine
creation rather than being dropped, so a policy never silently widens. Open
egress is direct on the user's own boxd account and the Everruns tenant egress
proxy does not see it.

## Tests

- Unit tests in each module.
- `tests/boxd_session_sandbox.rs` and `tests/boxd_tools.rs` run against an
  in-process mock (`tests/mock_boxd/`) that serves the key exchange and the
  gRPC API on one port, revokes the first JWT so every test exercises renewal,
  and keeps a small filesystem. A tonic server must implement every method of
  the 135-method service, so under `boxd-test-util` `build.rs` generates a
  second copy with the service cut down to the methods the client calls. They
  run in ci.yml.
- `tests/boxd_live.rs` (feature `boxd-live-tests`, `BOXD_API_KEY` from
  Doppler) runs in `boxd-integration.yml` on main and in the weekly live
  sweep: key validation, regions, exec, stdin, timeout, a 3 MiB binary file,
  a process surviving hibernation, labels, egress blocked then allowlisted,
  and the provider's full lifecycle.

## Not in v1

- **Computer use.** Every boxd machine has a display, but driving it needs a
  screenshot and input path over `Exec` that no live run has proven yet. The
  template refuses `desktop`.
- **`checkpointed` durability.** A boxd Disk outliving the machine could hold
  the workspace the way Daytona's recovery volume does, and boxd's native
  checkpoints could back rewind.
- **Snapshot builder.** Building a template's bootstrap into a boxd snapshot,
  like the Daytona builder. Snapshots made by hand already work.
- **Keyless GitHub.** boxd's host-bound secrets are org-level objects; one per
  sandbox needs a cleanup story first.
- **Region option for Daytona and Modal**, under the same `region` name.
