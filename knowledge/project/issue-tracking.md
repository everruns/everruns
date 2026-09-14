---
type: Specification
title: "Issue Tracking"
description: "Issue tracking (Linear, OSS project)."
tags:
  - everruns
  - project
---
# Issue Tracking

## Overview

We use [Linear](https://linear.app) for issue tracking. All issues for this repository belong to the **OSS** project.

- **Linear workspace:** Everruns
- **Team:** EVE
- **Project:** OSS

## Prerequisites

- Linear MCP server configured (agent global config)
- `LINEAR_API_KEY` available via Doppler
- GitHub CLI authenticated: `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'`

## Processing Issues

Use the [`/process-issues`](../../.agents/skills/process-issues/SKILL.md) skill to pick up, fix, and ship open issues. One PR per issue, up to 5 in parallel.

Before claiming an issue, check whether someone else is already working on it:

- issues already in `In Progress` with `updatedAt` within the last 1 day are considered actively owned and should not be auto-claimed
- issues already in `In Progress` with `updatedAt` older than 1 day require a human takeover decision; agents should raise them instead of silently reassigning or resetting them
- once an issue is available to pick up, the agent should move it to `In Progress` immediately before implementation starts

## Filing Issues

An issue must stand on its own for whoever picks it up, which may be an agent with
none of the context that produced it.

- State the problem, the evidence, and what done looks like in the issue body. Name
  the files and symbols involved. Someone should be able to start without asking
  what was meant.
- **A link is not context.** If an issue leans on a document, that document has to be
  reachable when the issue is read. A reference to a knowledge concept that has not
  merged yet is a dead link — land the concept first, or in the same change, or write
  the reasoning into the issue body instead.
- Prefer inlining the few sentences that matter over linking to where they live. Links
  rot, get filed against the wrong branch, or point at something the reader cannot see.
- Where an issue records a decision, record what was rejected and why. That is the part
  a reader cannot reconstruct and the part that stops the decision being re-argued.

This rule exists because it was broken: thirteen issues for the Slack modernization
work were filed pointing at a knowledge concept that had not been merged, so every
`Context:` link 404d for the entire time the work was being picked up.
