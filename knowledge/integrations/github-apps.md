---
type: Decision
title: "Per-agent GitHub Apps"
description: "Why a service virtual user connects to GitHub by creating its own GitHub App from a manifest, and why that one installation backs every GitHub consumer."
tags:
  - everruns
  - integrations
  - github
  - identity
---

# Per-agent GitHub Apps

## Abstract

An agent that reviews pull requests needs to read diffs, post comments, and
hear about new pull requests. Doing that with a personal token means copying a
secret and acting as a person. Instead, "Connect GitHub" on a service virtual user
creates a GitHub App for that agent and installs it on the repositories the
user picks. Everything is clicks; GitHub hands the App's credentials straight
to our callback.

## Decisions

- **One App per service virtual user.** The agent gets its own bot identity
  (`<app>[bot]`) and needs no operator setup on self-hosted deployments, the
  same trade the Slack one-click install made
  ([slack-one-click-install.md](slack-one-click-install.md)). A deployment-wide
  App would need an operator to register it and would make every agent the
  same bot.
- **Manifest flow, no copied secrets.** The browser posts a manifest to
  GitHub, GitHub creates the App and redirects back with a one-time code, and
  the server exchanges it for the App id, private key, client secret and
  webhook secret. The browser then continues to the App's install page, and
  GitHub's setup redirect records the installation.
- **One GitHub credential per service virtual user.** The installation is stored as the
  virtual user's `github` connection. The connection resolver mints installation
  tokens from that App's key, so native GitHub tools, git, MCP servers bound to
  the connection, and GitHub event triggers all act as the same installation.
  Current-input resolution selects the actual responder's active service account;
  the resolver prefers its App over the deployment-wide GitHub App.
- **The App row id is in every GitHub-facing URL** (setup, webhook), so a
  callback resolves its App without trusting request input, and a setup
  callback can only bind installations of that App.
- **Stateless round trips.** The browser carries an encrypted, expiring state
  bound to the org, service virtual user and management actor. The manifest callback
  rechecks the actor's live membership and connection authority. Setup verifies
  the App's installation and its active service virtual user.
- **Webhooks need a public URL.** GitHub refuses unreachable webhook URLs, so
  on a local base URL the manifest omits the webhook: tools still work, events
  do not.
- **Disconnect uninstalls, keeps the App.** GitHub has no API to delete an App;
  a reconnect reuses it.

- **One webhook per App, fanned out to triggers.** Deliveries arrive at
  `/v1/github/apps/{app_row_id}/webhook`, are verified against that App's
  webhook secret, and go to the GitHub triggers of agents on the App's service virtual user
  (`knowledge/runtime-resources/agent-triggers.md`). An `installation.deleted`
  delivery removes the virtual user's connection.

## Where it lives

- Service: `crates/server/src/github_apps.rs`
- Routes: `crates/server/src/api/github_apps.rs`
- Token resolution: `crates/server/src/storage/connection_resolver/mod.rs`
- Threats: TM-GHAPP in [threat-model.md](../security/threat-model.md)
