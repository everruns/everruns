---
type: Test Case
title: "TC001: Connect a Coding Agent"
description: "Verify the coding-agent setup dialog preserves the current deployment and keeps runtime provider setup separate."
tags:
  - everruns
  - test-case
  - ui
  - onboarding
---

# TC001: Connect a Coding Agent

## Preconditions

- A local no-auth stack and an authenticated stack.
- An organisation with no agents, and a new organisation entering provider setup.

## Steps

1. Open Connect your coding agent in the user menu. Verify no host or organisation selector is shown.
2. Copy the setup prompt. Verify it contains the public bootstrap URL, this deployment's exact MCP endpoint and its authentication guidance.
3. Verify Claude Code, Codex, Cursor and Gemini CLI icons and labels appear without a selector, configuration accordion or separate authorization section.
4. Open Manual setup and verify the maintained AI-tools installation guide is reached. Manage OAuth approvals separately in Settings → My agent experience → Connected AI clients; refresh its approvals without a plugin-installation or setup-success claim.
5. On the no-auth stack, verify the prompt states that authentication is not required. On the authenticated stack, verify the prompt retains browser OAuth guidance.
6. Open the empty Agents page and verify its connection action opens the same dialog.
7. In organisation Configure, skip provider setup. Verify Done offers Add a model provider linking to Models → Providers, explains that Platform Chat requires it, and still offers coding-agent connection.
8. With an existing provider, continue through Configure without entering another key. Verify Done offers Platform Chat as primary and coding-agent connection as secondary.
9. Repeat the dialog at a mobile viewport, with keyboard navigation, Escape and clipboard access denied. Verify scrolling, focus and manual-copy feedback remain usable.

## Expected Result

- One shared, deployment-aware connection flow is reachable without a feature flag.
- OAuth approvals and local plugin installation are not conflated.
- Provider setup remains in the existing onboarding arc and is actionable after a skip.
- Setup instructions contain no credentials and do not run or create agents as a connection check.
