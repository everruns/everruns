---
type: Specification
title: Worker history cache
description: A gRPC worker keeps each session's message history and reads only the messages after the last sequence it holds.
tags:
  - execution
  - performance
  - worker
---

# Worker history cache

## Problem

Every reason step on a gRPC worker read the session's whole message history
through `LoadMessages`. The server read every message event, rebuilt each
message, encoded it, and counted the session's message events. A step's server
cost grew with the length of the session, though at most a few messages were
new.

## Decision

The worker's gRPC client (`crates/worker/src/grpc_adapters/history_cache.rs`)
keeps each session's history with the event sequence it was read up to. An
unfiltered read asks the server only for messages after that sequence
(`LoadMessagesRequest.after_sequence`) with `skip_total_count`, appends them,
and cuts the latest-N window and every filter from the cached list, exactly as
from a full read.

This is correct whoever wrote the new events, so the cache needs no lease
tracking:

- History on this path is append-only: it is derived from `input.message`,
  `output.message.completed` and `tool.completed` events, which are never
  edited.
- One session's events commit in sequence order: the batch insert reserves
  sequences by updating the session's `event_sequences` row, whose lock holds
  until commit, so a later sequence is never visible before an earlier one.

## Limits

- Bounded by wire size (64 MiB per worker), least recently used first out.
- A cold "latest N" read stays a windowed server read; warming the cache would
  pull the whole history for one window. A cold full read warms it.
- A failed read drops the session's entry. Two racing reads never step an
  entry back to an older sequence.
- `WORKER_HISTORY_CACHE=off` disables it.

## Verification

- Unit: `history_cache.rs` tests (append, no step back, disabled, forget).
- End to end over gRPC: `crates/server/src/worker_link/grpc_service/tests/history_cache.rs`
  compares the cached read with a fresh client's read as a session grows,
  including the latest-N window.
