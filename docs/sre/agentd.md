---
title: Data plane (agentd)
description: What agentd, the data plane that runs your agents' sessions, promises about your data, how to choose a setup for it, and how to watch, check and restore it.
sidebar:
  order: 20
---

agentd is where your agents' sessions live and run. Docker Compose, the Helm
chart and `everruns docker up` start it by default (data plane v2); the API
keeps everything else: agents, people, organizations, settings and billing.
One image, `ghcr.io/everruns/agentd`, runs three roles:

| Role | Does | Keeps state |
| --- | --- | --- |
| Shard node | Holds sessions: their messages, events, workspace files and SQL databases. A shard is one node, or three that keep copies of each other. | Its data directory (`/data`) and an object store |
| Executor | Runs turns: calls the model and the tools, and writes each step back to the shard. | None; a turn it was running continues on another executor if it stops |
| Edge | The one address the proxy sends session calls to: sends a session's calls to its shard and everything else to the API. | None |

agentd also runs on its own, without the API, as the backend for your own
app's agents: see [Run agents with agentd](/getting-started/agentd/).

This page is about what agentd promises and how to keep those promises in
your install. Setting it up is in the [Docker Compose](/getting-started/docker-compose/#data-plane)
and [Kubernetes](/getting-started/kubernetes/#choose-the-data-plane) guides,
and its variables in [Environment Variables](/sre/environment-variables/#data-plane-v2).

## What agentd promises

When agentd answers a call (a message sent, a session created, a file
written), the write is **committed**: written and flushed to disk on the
shard's node, or on two of its three nodes. agentd never answers before
that, and nobody (a stream, a listing, the API) ever sees a write that is
not committed.

After that, the shard copies its log to the **object store** (S3 or
a compatible store, or a directory) every 10 seconds. The object store is
the disaster copy: what a shard is rebuilt from when its disks are gone,
and what point-in-time restores read.

What a failure costs depends on how many nodes the shard has and where the
object store is:

| Failure | One-node shard | Three-node shard |
| --- | --- | --- |
| agentd restarts or crashes | Nothing is lost | Nothing is lost; another node takes over within seconds |
| One node's disk is lost | The last 10 seconds of writes, at most 30 | Nothing is lost |
| Two nodes' disks are lost | | Usually nothing: the third node's disk holds what it had received |
| Every disk of the shard is lost | The last 10 seconds of writes, at most 30 | The last 10 seconds of writes, at most 30 |
| The object store is on the lost disk or machine | Everything | Everything |

The 30 seconds is a hard bound: a shard whose copying falls more than 30
seconds behind stops taking new work until it catches up.

Some writes are never in that window, because losing them would do more
than lose recent history: a session's creation, a session's move between
shards, and the record that a tool call which must not run twice has
started. agentd answers these only once every node of a three-node shard
has them, or once the object store has them. So:

- **A tool call that must run at most once never runs twice**, through any
  failure that leaves one disk or the object store. If the call's outcome
  was lost, the agent is told the outcome is unknown; the call is not run
  again on its own. Each call carries an idempotency key that stays the
  same on every attempt (see [Durable execution](/explanation/durable-execution/#guarantees)).
- **A session is never lost from the directory** that finds its shard.

What else holds:

- **One writer per session.** An executor that lost a session (it stalled,
  or the session moved) cannot write to it again.
- **Turns survive.** A turn continues after its executor stops or its
  shard's leading node changes, from the session's own log.
- **Streams have no gaps.** A session's event stream resumes from its last
  event id with no gap and no repeat, also after a node restart or a move.
- **File and SQL reads are current.** A read after a write sees the write,
  whichever node answers.
- **Spend stays within the balance.** With the API unreachable, running
  turns use the allowance they hold, then wait.
- **Overload is refused, not crashed into.** Near its memory limit a shard
  takes no new sessions, then answers new messages with 429 and a
  `Retry-After`, then (at 95%) starts no new turns; turns already running
  finish. A node that still reaches 98% restarts itself before the kernel
  would kill it (exit code 75), and only while the rest of its shard is up.
  An executor takes turns by its free memory and hands its newest turns to
  another executor at 95%.
- **Large payloads stay out of memory.** A message, input or event value
  over 256 KB is kept in the object store and read back whole by the
  readers that need it.

What agentd does not promise:

- That a write someone already saw survives losing every disk: it may fall
  in the last 10 seconds (at most 30).
- That a tool which declares itself idempotent runs once: after a failure
  it runs again, and should forward its idempotency key so the repeat is
  harmless.
- Latency. agentd reports its own latency (below) but does not bound it.

## Choose a setup

- **Keep the object store off the shard's machine.** The Compose default,
  `file:///store` on the `agentd_store` volume, is on the same machine as
  the shard: a lost machine loses both. Set `AGENTD_STORE=s3://bucket/prefix`
  (with the usual `AWS_*` settings) in `.env`; the Helm chart uses
  `storage.s3`'s bucket once [S3 is on](/getting-started/kubernetes/#keep-files-in-s3).
- **Run three shard nodes to survive a lost node or disk** with no loss and
  no downtime: `dataPlane.shard.nodes=3` in Helm, on three machines, ideally
  in three zones. One node is fine for a single-machine install.
- **Give the node memory to spare.** Set the container's memory limit about
  1.5 times the memory budget (`AGENTD_MEMORY_BUDGET_MB`,
  `dataPlane.shard.memoryBudgetMb`): a snapshot briefly needs the headroom.
- **Check a new object store before you rely on it:**

  ```bash
  docker compose run --rm --no-deps agentd store-check
  ```

  It tries the store operations restores depend on, under a scratch prefix
  it removes after, prints what it found as JSON and fails if any check
  does. Some S3-compatible stores differ from S3 in exactly these
  operations.

## Move organizations from v1

Upgrading an install that ran before data plane v2 leaves each organization
with sessions on the workers (v1); only new organizations start on agentd.
To move existing ones, run `dataplane move-orgs` from the admin image,
against the install's database, with the server's keys. Each organization is
switched to agentd, then its sessions move one at a time with their
conversation, values, secrets and files; a session in the middle of a turn
is skipped. Moved sessions keep their ids, so nothing outside notices.

```bash
# Docker Compose: on the stack's network, with its .env
docker run --rm --network everruns_default --env-file .env \
  -e DATABASE_URL=postgres://everruns:everruns@postgres:5432/everruns \
  ghcr.io/everruns/everruns-admin:$EVERRUNS_TAG \
  dataplane move-orgs local <org id> <org id> ...
```

On Kubernetes, run the same image as a Job with the release's database URL
and `everruns` Secret (`SECRETS_ENCRYPTION_KEY`, `EVERRUNS_DP_TOKEN`,
`EVERRUNS_DP_KEY`); the data plane's id is `local` there too. Pass `-` instead
of ids to read them from standard input, one per line. The command prints a
line per organization and fails if any session did not move; run it again to
pick those up. `dataplane move <org id>` moves one organization, and
`dataplane status <org id>` shows where one runs.

Start with a test organization, then a few small ones, then the rest. The
`everruns_default` network is the Compose guide's (a project in a folder named
`everruns`); use your project's name if it differs.

To keep new organizations on v1 for now, set `EVERRUNS_DATA_PLANE=v1`.
Organizations agentd already took before that and that never ran a session
there go back with `dataplane to-v1 local` (every such organization) or
`dataplane to-v1 local <org id> ...`. One that ran sessions on agentd stays:
the command lists it with its session count, because sessions do not move
back to v1.

## Watch it

Each shard node and executor serves Prometheus metrics at `/metrics`. The
ones to alert on:

| Metric | Means | Worry when |
| --- | --- | --- |
| `agentd_shard_slo_ship_lag_p99_seconds` | How long the oldest write waited to reach the object store | Above 15 s; at 30 s the shard stops taking new work |
| `agentd_shard_slo_admit_p99_seconds` | How long taking a message took | Above 100 ms |
| `agentd_shard_slo_ship_first_p99_seconds` | How long the writes that must survive any disk loss waited | Above 100 ms |
| `agentd_shard_overload_level` | 0 normal, 1 full (no new sessions), 2 shedding (new messages refused), 3 emergency (no new turns) | Stays at 2 or above: add memory or a shard |
| `agentd_memory_actions_total{action="restart"}` | Restarts a shard node did at its memory limit (an executor exits with code 75 instead) | Any: memory is too tight for the load |
| `agentd_memory_oom_kills_total` | Restarts after the kernel killed the process at its limit | Any |
| `agentd_memory_bytes{source}` | The node's memory: `events` and `raft_log` (counted, capped by `agentd_memory_cap_bytes`), `unaccounted` live heap, allocator `fragmentation` | `unaccounted` above 15% of the limit |
| `agentd_shard_commit_stalled` | 1 while writes are not committing | Lasts minutes: nodes are down or partitioned, or the disk stalls |
| `agentd_shard_catching_up` | 1 while a node replays the log after a restart | Lasts long: that node is not a full copy, so the shard survives one fewer failure |

To see where a session is and how each shard keeps up (read only):

```bash
docker compose exec agentd /app/agentd inspect shards
docker compose exec agentd /app/agentd inspect session <session id>
```

## Restore

The object store keeps every moment of the last 7 days restorable
(`AGENTD_RESTORE_WINDOW_HOURS` changes the window). A restore rebuilds the
shard into empty data directories as a new incarnation; nothing from before
the rebuild (a late message from a stopped executor, say) is accepted after
it.

Stop the shard's nodes and its executors first. Always look before you
write: `--dry-run` reports what the restore would find and writes nothing.

**A one-node shard whose disk was lost** (a new, empty `agentd_data` volume):

```bash
docker compose stop agentd executor
docker compose run --rm --no-deps agentd restore --shard 0 --node 1=/data --dry-run
docker compose run --rm --no-deps agentd restore --shard 0 --node 1=/data
docker compose up -d agentd executor
```

**A three-node shard that lost two nodes:** give the stopped survivor's data
directory, so nothing it held is lost, and an empty directory for each node:

```bash
agentd restore --shard 0 --survivor /mnt/survivor-data \
  --node 1=/data/1 --node 2=/data/2 --node 3=/data/3
```

**Back to a moment**, for example before a bad upgrade: add `--at` with an
RFC 3339 time. Everything after that moment is gone from the shard.

```bash
agentd restore --shard 0 --at 2026-11-22T14:00:00Z --node 1=/data --dry-run
```

**One organization only**, on a running data plane, leaving every other
organization as it is:

```bash
agentd restore-org --org <org id> --at 2026-11-22T14:00:00Z
```

In the container the binary is `/app/agentd-testnode`; Compose and Helm
start it with the right `AGENTD_TOPOLOGY` and `AGENTD_STORE`, which every
restore command needs as well.

## How it is tested

Every promise on this page has a test that fails when the promise breaks:
crash tests of the write log, models of the move, spend and durability
protocols, tests that kill nodes and executors in a running data plane,
a check of the real object store, and a weekly drill that loses a whole
shard under load and rebuilds it. What each covers, and what is not tested
yet, is in the repository:
[guarantees](https://github.com/everruns/everruns/blob/main/knowledge/execution/data-plane-v2/guarantees.md)
and [testing](https://github.com/everruns/everruns/blob/main/knowledge/execution/data-plane-v2/testing.md).
