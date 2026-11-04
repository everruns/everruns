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

1. Open the Agent and, in the **More** list of the **Agent** tab, select **GitHub**.
   It opens the Agent's GitHub page, `/agents/{agent_id}/github`.
2. Open
   **Create this agent's own GitHub App** and choose where to create
   the App: **Personal account**, an organization your Everruns organization
   already connected to GitHub, or **Another organization…** to type one. The
   App is private, so it can only be installed on the account that owns it.
3. GitHub shows the App it is about to create for this Agent, named after the
   Agent (with a short random suffix when another App already has that name).
   Select **Create GitHub App**.
4. Choose where to install it and which repositories it can reach, then select
   **Install**.

You land back on the Agent's **GitHub** page showing the
Agent's bot login (for example `pr-summarizer[bot]`), the account and the
repositories. The App is this Agent's alone: its comments appear as the App's
bot. Its GitHub tools, its trigger, and any MCP server set to use the Agent's
`github` connection all use this one installation.

### Set the avatar

GitHub offers no way to set an App's logo for it, so the **GitHub** page
shows a **Set the avatar** step. Select **Download avatar** to save the Agent's
avatar (shown when the Agent has one), then **Open App settings** and upload
it as the logo on GitHub.

### Use the Everruns app instead

When an org admin has linked a GitHub account to the shared Everruns GitHub
App, the **GitHub** page also offers **Use the Everruns app**. Pick a
linked account and the Agent acts as the shared App's bot on that
installation, with no App of its own to create. Its comments then appear as
the shared bot, not as the Agent, and there is no avatar step.

To link an account, an org admin opens **Settings > Organization**, selects
**Link** in the **Everruns GitHub App** group, and installs the App on GitHub.
**Unlink** there removes the account from every Agent that chose it; the App
stays installed on GitHub.

## Add a pull request trigger

1. On the Agent's **GitHub** page, select **Add pull request trigger**.
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

To run the trigger on a pull request that is already open, without pushing to
it, fire it with a reference to the pull request. The run continues that pull
request's own session, as a real delivery would:

```bash
everruns agents triggers trigger --agent-id agent_01h9 --trigger-id trg_01h9 \
  --event '{"repository":"acme/api","pull_request":12}' --reason 'Summarize this pull request'
```

See [Fire a trigger with an event](/features/agent-triggers/#fire-a-trigger-with-an-event).

## Disconnect

Select **Disconnect** on the Agent's **GitHub** page. This uninstalls the App from
GitHub and removes the Agent's GitHub connection. For an Agent using the
Everruns app, it only removes the Agent's choice; the account stays linked to
the organization. The App itself stays on your
GitHub account, so connecting again reuses it; delete it from GitHub's
**Developer settings** if you no longer want it.

If the App was created but never installed, for example on your personal
account when the repositories belong to an organization, select **Start over**
in the **GitHub** page's actions menu instead. Everruns forgets that App, so the next **Connect GitHub** creates a new
one.

## From the CLI

The same connection is available from the `everruns` CLI, and to agents through
the Everruns MCP server:

```bash
# Is the App created, installed, and on which account?
everruns agents github get --agent-id agent_01h9

# The install link, once the App exists. Creating the App itself needs the
# web UI, because GitHub confirms it in the browser.
everruns agents github connect --agent-id agent_01h9 --reason 'Add the API repo'

# Uninstall and remove the connection, or start over with an App that was
# never installed.
everruns agents github disconnect --agent-id agent_01h9 --reason 'Reviewer retired'
```
