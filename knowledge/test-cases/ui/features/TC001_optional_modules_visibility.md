---
type: Test Case
title: "TC001: Optional module visibility"
description: "Verify that Evals, Skills, Memory, and Plugins stay hidden until their organization feature flags are enabled."
tags:
  - everruns
  - test-case
  - ui
  - features
---
# TC001: Optional module visibility

## Description

Verify that Evals, Skills, Memory, and Plugins stay hidden until their
organization feature flags are enabled.

## Preconditions

- Server running with the four deployment feature flags available
- User logged in as an organization owner or admin
- The organization's `evals`, `skills`, `memory`, and `plugins` flags are disabled

## Test Data

| Field | Value |
|-------|-------|
| Features | Evals, Skills, Memory, and Plugins |

## Steps

1. Confirm the four features are absent from the sidebar and global search.
2. Navigate directly to each feature URL and confirm the disabled state links to Settings → Features.
3. Open Settings → Features and enable all four features.
4. Confirm all four sidebar entries and global-search results appear.
5. Open each feature URL and confirm its management page loads.

## Expected Result

| Check | Expected |
|-------|----------|
| Disabled navigation | No sidebar or global-search entry for any of the four features |
| Disabled direct route | Disabled state appears without mounting the feature page |
| Feature settings | An owner or admin can enable each available feature |
| Enabled navigation | All four sidebar and global-search entries appear |
| Enabled direct route | Each management page loads normally |
