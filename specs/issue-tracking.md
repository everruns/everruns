# Issue Tracking

## Overview

We use [Linear](https://linear.app) for issue tracking. All issues for this repository belong to the **OSS** project.

- **Linear workspace:** Everruns
- **Team:** EVE
- **Project:** OSS

## Prerequisites

- Linear MCP server configured (`.mcp.json`)
- `LINEAR_API_KEY` available via Doppler
- GitHub CLI authenticated: `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'`

## Processing Issues

Use the [`/process-issues`](../.claude/skills/process-issues/SKILL.md) skill to pick up, fix, and ship open issues. One PR per issue, up to 5 in parallel.

Before claiming an issue, check whether someone else is already working on it:

- issues already in `In Progress` with `updatedAt` within the last 1 day are considered actively owned and should not be auto-claimed
- issues already in `In Progress` with `updatedAt` older than 1 day require a human takeover decision; agents should raise them instead of silently reassigning or resetting them
- once an issue is available to pick up, the agent should move it to `In Progress` immediately before implementation starts
