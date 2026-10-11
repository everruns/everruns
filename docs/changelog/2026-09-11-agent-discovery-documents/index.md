---
title: Everruns servers publish an MCP server card and auth.md
description: Every Everruns deployment now serves an MCP Server Card and an auth.md document, derived from live config, so agents can discover how to connect.
date: 2026-09-11
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [agent, api]
release: "0.26.0"
links:
  docs: [/sre/runbooks/authentication/]
  prs: [everruns/everruns#3533, everruns/everruns#3530]
---

An Everruns server now describes itself to agents. `/.well-known/mcp/server-card.json`
is an MCP Server Card (SEP-1649), and `/auth.md` explains both credential paths:
OAuth for the MCP endpoint and personal access tokens for the REST API. Both are
generated from the running server's configuration, so a self-hosted deployment
describes itself correctly. Under `AUTH_MODE=none` they say no credentials are
required.

The descriptions of `create_agent`, `create_session` and `create_message` also
state their argument constraints, so an agent gets them right on the first call.

See [Authentication](/sre/runbooks/authentication/). [#3533](https://github.com/everruns/everruns/pull/3533), [#3530](https://github.com/everruns/everruns/pull/3530)
