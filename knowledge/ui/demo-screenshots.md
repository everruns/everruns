---
type: Playbook
title: Demo Screenshot Set
description: Defines the maintained Everruns demo screenshot scenes, data, framing, and refresh contract.
tags:
  - everruns
  - ui
  - demo
  - screenshots
---

# Demo Screenshot Set

Everruns keeps a small, curated screenshot set in [`assets/screenshots/`](../../assets/screenshots/)
for demos, announcements, and product overviews. These files are product assets and are committed.
They are distinct from transient PR evidence, which is uploaded to GitHub and never added to the
repository.

## Capture contract

Every scene has matching light and dark PNGs rendered from a 1440 by 900 CSS-pixel viewport at a
device pixel ratio of 2. The resulting 2880 by 1800 assets preserve the intended desktop composition
while providing enough detail for Retina displays and cropping. The pair must show the same
application state, use the standard sidebar, contain no browser chrome, and avoid hover states,
menus, loading indicators, error overlays, secrets, or personal data. Capture from a real local
stack after the page reaches its stable state; do not mock or paint the product UI.

The maintained set is:

| Scene | Route | What the frame demonstrates |
|---|---|---|
| Platform Chat | `/chats/{platform_chat_session_id}` | A completed conversation that creates ten imaginative agents, including the confirmation boundary and linked result list. |
| Sessions | `/sessions` | A populated operational overview with chat and API sessions, filters, saved views, and aggregate facets. |
| Agents | `/agents` | A varied ten-agent catalog whose cards include descriptions, status, activity, and explicit harness inheritance. |
| Harnesses | `/harnesses` | The four built-in harnesses, their inheritance, capability density, tags, and status facets. |
| Durable Execution | `/durable` | A healthy worker plus workflow, task, throughput, and system-load telemetry. |

Stable filenames use `<scene>-light.png` and `<scene>-dark.png`; changing a scene requires replacing
both variants. Durable Execution is intentionally included alongside the primary catalog pages: its
charts make the runtime dimension of the product visible instead of presenting Everruns as only a
configuration UI.

## Demo state

Start the canonical local stack with session sandboxes enabled:

```bash
PORT_PREFIX=271 AUTH_MODE=none FEATURE_SESSION_SANDBOX=true ./scripts/start-agent-dev.sh
```

Use Platform Chat to send `Create 10 agents, imagine something`, then approve with `Yep`. The scene
expects these active agents: Idea Forge, Research Scout, Story Architect, Design Critic, Code
Companion, Data Detective, Workflow Optimizer, Learning Coach, Customer Voice, and Chaos Tester.
Keep the completed exchange visible for the Platform Chat capture.

Seed five idle API sessions so the operational view has a compact, legible cross-section:

| Session title | Agent | Goal |
|---|---|---|
| Launch Brief | Idea Forge | Create a crisp launch concept for an AI developer platform. |
| Competitive Research | Research Scout | Map the market and identify differentiated opportunities. |
| Onboarding Review | Design Critic | Review the onboarding flow for clarity, confidence, and momentum. |
| Customer Signals | Customer Voice | Synthesize customer feedback into themes and actionable insights. |
| Workflow Tune-up | Workflow Optimizer | Reduce friction in the support-to-engineering handoff. |

Create each session with `POST /api/v1/sessions`, using its agent's registered `agent_name`, the
table's `title` and `goal`, and `source: "api"`. Do not send a message: the idle state is deliberate
because it keeps the session list readable. The existing Platform Chat session provides the sixth,
completed row.

Allow the worker to run long enough for `/durable` to report healthy and draw task throughput. The
exact counters and timestamps are live telemetry and may change; the stable requirement is one
accepting worker, no failures, and visible chart activity.

## Capture and review

The capture implementation lives with the repository screenshot workflow rather than duplicating
browser commands here:

```bash
.agents/skills/ui-screenshots/scripts/capture-demo-screenshots.sh \
  http://localhost:27100 assets/screenshots
```

The script resolves the current pinned Platform Chat session through the sessions API, so local
session IDs never become part of the asset contract.

Before committing, inspect all ten images at original resolution. Confirm paired scene state,
correct theme, 2880 by 1800 output dimensions, the unchanged 1440 by 900 CSS composition, readable
primary content, no clipped page header, no dev overlay, and no sensitive values. The capture script
asserts the PNG dimensions after every capture and removes Next.js' development-only portal from the
frame; therefore these assets are presentation material, never debugging or regression-test
evidence.

## Refresh policy

Refresh the set when navigation, visual identity, core cards, or one of the represented pages
changes materially. Prefer replacing an existing scene over growing the set without a distinct
demo story. Add a new scene only when it communicates a major product dimension not already visible
in the five maintained views.
