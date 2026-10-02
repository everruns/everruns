---
title: Summarize GitHub Pull Requests
description: Connect GitHub to an Agent, subscribe it to pull request events, and have it keep one summary comment on every pull request up to date.
appliesTo: [platform, cloud]
---

This guide builds an Agent that comments a summary on every pull request in the
repositories you choose, and updates that comment when new commits are pushed.
Setup is all clicks: the Agent gets its own GitHub App, so there is no token to
create or paste.

## Prerequisites

- An active Agent.
- A public HTTPS Everruns origin. GitHub only delivers webhooks to a public URL;
  on a local origin the App is still created, and its tools work, but no events
  arrive.
- Permission to create a GitHub App on your account or organization, and to
  install it on the repositories you want summarized.

## Give the Agent the GitHub tools

1. Open the Agent and select **Edit**.
2. Add the [GitHub](/capabilities/github/) capability. It gives the Agent
   `get_github_pull_request`, `get_github_pull_request_diff` and
   `upsert_github_comment`.
3. Save.

## Connect GitHub

1. Open the Agent and select **Integrations**.
2. On the **GitHub** card, select **Connect GitHub**.
3. GitHub shows the App it is about to create for this Agent. Select **Create
   GitHub App**.
4. Choose where to install it and which repositories it can reach, then select
   **Install**.

You land back on **Integrations** with the card showing the account and
repositories. The App is this Agent's alone: its comments appear as the App's
bot. Its GitHub tools, its trigger, and any MCP server set to use the Agent's
`github` connection all use this one installation.

## Add a pull request trigger

1. On the **GitHub** card, select **Add pull request trigger**.
2. Keep the default events (opened, reopened, new commits pushed, ready for
   review), or change them.
3. Leave repositories empty to cover every repository the App is installed on,
   or pick some.
4. Keep **One session per pull request**, so every push to a pull request
   continues the same conversation.
5. Review the message. The default asks the Agent to read the pull request and
   its diff and to post or update its summary. Values such as
   `{{github.repository}}`, `{{github.number}}`, `{{github.title}}` and
   `{{github.url}}` are filled in from the event.
6. Select **Save**.

## Verify

1. Open a pull request in one of the repositories.
2. Within a few seconds the trigger's history on the Agent shows a
   `dispatched` delivery, and a session starts.
3. The Agent posts a comment on the pull request. Push another commit: the same
   comment is edited instead of a new one being added.

Deliveries that were filtered out (for example, a repository outside the
trigger's list) or deduplicated (a GitHub redelivery) are listed with the
reason, so you can see why an event did not start a run.

## Disconnect

Select **Disconnect** on the **GitHub** card. This uninstalls the App from
GitHub and removes the Agent's GitHub connection. The App itself stays on your
GitHub account, so connecting again reuses it; delete it from GitHub's
**Developer settings** if you no longer want it.
