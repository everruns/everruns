---
title: Set up review and security agents
description: Adopt the PR Reviewer and Security Scanner templates to review every pull request with inline comments and scan a repository for vulnerabilities on a schedule, on any model.
---

Two agent templates turn the [GitHub](/capabilities/github/) capability into
working automation in a few clicks:

- **PR Reviewer** reviews every pull request on a repository with inline
  comments and keeps one summary comment current. After a new push it adds only
  what is new.
- **Security Scanner** clones a repository into a cloud sandbox every week,
  looks for vulnerabilities, and files each confirmed finding as a GitHub issue,
  updating the same issue on later scans instead of opening duplicates. If you
  allow it, it also opens draft pull requests for small, high-confidence fixes.

Both run on whichever model the agent uses. Each gets its own GitHub App, so it
reads code and posts as its own bot, with no token to create or paste.

## Prerequisites

- A public HTTPS Everruns origin for the PR Reviewer. GitHub only delivers
  webhooks to a public URL. The Security Scanner runs on a schedule and does not
  need one.
- Permission to create a GitHub App on your account or organization, and to
  install it on the repository.
- For the Security Scanner, a [Daytona](/integrations/daytona/) API key.

## Adopt a template

1. Open **Agents** and find **PR Reviewer** or **Security Scanner** in the
   examples. Templates carry a **guided setup** badge.
2. Select **Import**. The agent is created and the setup page opens.

## Connect GitHub

1. On the setup page, select **Connect GitHub**.
2. GitHub shows the App it is about to create for this agent. Select **Create
   GitHub App**, then choose where to install it and which repositories it can
   reach, and select **Install**.

You land back on the setup page with GitHub marked as connected.

## Pick a repository and settings

1. Pick the repository. When the App is installed on exactly one repository it
   is picked for you.
2. Security Scanner only:
   - **File findings on private repositories only** is on by default. On a
     public repository an issue would disclose the vulnerability, so with this
     on the scanner reports the finding only in its run instead.
   - **Open fix pull requests** is off by default. Turn it on to let the
     scanner open draft pull requests. Pushing a branch also needs **Contents:
     write** on the agent's GitHub App, which agent Apps do not request: grant
     it under the App's **Permissions & events** on GitHub and accept the change
     on the installation. Without it the scanner says the push was refused and
     carries on.
   - Add the Daytona API key to the agent's service account (the setup page
     links to it), since scheduled runs act as the agent.
3. Select **Create trigger**.

The PR Reviewer gets a GitHub trigger for pull requests opened, reopened,
pushed to, or marked ready for review, skipping drafts, with one session per
pull request. The Security Scanner gets a schedule trigger every Monday at
06:00 UTC. Both triggers can be edited afterwards from the agent's
**Integrations** tab, like any other trigger.

## Verify

PR Reviewer:

1. Open a pull request in the repository.
2. The trigger's history shows a `dispatched` delivery and a session starts.
3. The agent submits a review with inline comments, or none when it finds
   nothing, and posts a summary comment. Push another commit: the summary is
   edited, and only new findings are added as comments.

Security Scanner:

1. On the agent's **Integrations** tab, select **Run now** on the trigger instead of waiting
   for Monday.
2. The session clones the repository, scans it, and ends with a report.
   Findings appear as issues labelled `security`. Run it again: existing
   issues are updated, not duplicated, and an issue you closed stays closed.

## What the agents cannot do

- The reviewer cannot approve a pull request. It comments or requests changes.
- Pull request text, diffs, code and issues are written by others and treated
  as untrusted. The agents are told to ignore instructions in them, and the
  settings above are enforced by the tools rather than by the prompt.
- Without **Open fix pull requests** the scanner has no tool to open a pull
  request at all.
