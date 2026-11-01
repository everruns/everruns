---
type: Decision
title: "Session Cost Tab"
description: "Why the session Cost tab leads with spend split by cost category, how that split is derived when only a call's total cost is stored, and what the tab leaves out."
tags:
  - everruns
  - ui
  - sessions
  - cost
---

# Session Cost Tab

Status: approved in the project thread on 2026-11-01; shipped with the `GET
/v1/sessions/{session_id}/cost` read.

## Problem

The old tab showed token totals, failure counts, a definition block and the context
report. It never showed money, though every model call stores its cost. The definition
block repeated the agent and model from the session header, and the model is not fixed
for a session. The per-source context list ran to dozens of rows of `0%`. Nothing showed
when the spend happened or what caused it. A session that a five-minute schedule kept
alive for 13 days spent $106, and the tab gave no hint of it.

## Decisions

- **Lead with spend by cost category.** The categories are uncached prompt, prompt read
  from cache, prompt written to cache, and output. Each row shows tokens, the average
  rate, the spend and its share. They show where money goes and what caching saved.
  Model breakdowns are rare (most sessions use one model). The model is named in the
  card header, and a Models table appears only when a session used more than one.
- **Split each call by the price table, then scale to what was billed.**
  `llm_generations` stores a call's tokens and its total cost (provider-reported where
  available, otherwise the price-table estimate), not a cost per bucket. The server
  prices each call's buckets with the model's price table at that call's prompt size,
  then scales the parts so they add up to the recorded cost. Every total on the page
  then equals what was billed. A call with a reported cost but no price table keeps its
  cost as "not split by category" rather than a guessed split. Logic:
  `crates/server/src/domains/session_cost/commands.rs`.
- **Spend over time by day or by turn.** The server returns UTC hours, and the UI rolls
  them into days in the viewer's timezone, so one response serves every viewer. Sessions
  that span two days or less default to the per-turn view, which shows the cost of each
  turn growing as the conversation grows, with failed turns marked underneath.
- **Usage that is not a token category is shown as usage, not dollars.** Tool calls
  (calls, failures, running time per tool) come from the trace index. Sandbox running
  and paused time comes from `sandbox_state_transitions`. Sandbox dollars and hosted
  tool charges are not recorded per session, so the tab does not invent them.
- **Who started the turns** groups turns and spend by the input message's initiator
  (person, schedule, agent trigger, channel) and names the session schedule. A notice
  appears when most spend is prompt that was not read from cache. It names the schedule
  when one schedule caused most of it.
- **The definition block is gone.** The harness, agent revision, identity and fork origin,
  which the header does not show, move to a one-line footer.
- **Context now** keeps the section breakdown. The per-source list is folded behind one
  summary line and opens into a table of the top sources.

## Bounds

The read never loads `llm.generation` payloads, which carry whole requests. It reads one
narrow row per model call, one per turn and one per tool name, and returns at most the
newest 5,000 turns (`turns_truncated` says when older ones were left out).

## Follow-ups

- Record sandbox cost per provider rate so sandbox time can carry dollars.
- Store hosted-tool charges (`cost_components`) per generation row so they can join the
  category table.
- When message origin metadata lands, use it for the origin labels instead of the
  event initiator.
