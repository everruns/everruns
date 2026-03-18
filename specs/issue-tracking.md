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
