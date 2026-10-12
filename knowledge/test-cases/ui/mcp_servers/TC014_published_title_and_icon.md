---
type: Test Case
title: "TC014: Published MCP title and icon"
description: "Verify that a catalog server shows the title and icon it publishes, and keeps the operator slug."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC014: Published MCP title and icon

## Description

Verify that a catalog server shows the title and icon it publishes, and keeps the operator slug.

## Preconditions

- API server is running
- An MCP server exists whose remote endpoint publishes a title (server card or protected-resource `resource_name`)

## Test Data

N/A

## Steps

1. Navigate to Settings > Organization > MCP catalog
2. Find the server

## Expected Result

- The heading is the published title
- The operator slug appears under the heading
- The operator description is unchanged
- An icon appears only when the server publishes a same-origin image; otherwise the generic MCP mark remains
