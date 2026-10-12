---
title: Connect several Slack workspaces per organization
description: An organization connects its Slack workspaces once in Settings, and adding an agent to Slack becomes picking a workspace and approving one consent screen.
date: 2026-10-01
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.34.0"
visual: "Settings > Slack workspaces with two connected workspaces, and a Slack channel editor showing Live in Slack with an Open in Slack link; crop to each panel."
links:
  docs: [/capabilities/slack/, /how-to/publish-to-slack/]
  prs: [everruns/everruns#3830, everruns/everruns#3977]
---

An organization administrator connects a Slack workspace once from
**Settings > Slack workspaces** by pasting a Slack configuration token. An
organization can connect more than one workspace.

Each agent you put in Slack gets its own Slack app. In the agent's Slack
channel editor, choose a workspace if there is more than one and select **Add to
Slack**. Everruns creates the app and opens Slack's consent screen with that
workspace selected. The channel then shows **Live in Slack** with an **Open in
Slack** link.

See [Connect a Slack workspace](/capabilities/slack/#connect-a-slack-workspace)
and [Publish to Slack](/how-to/publish-to-slack/). PRs [#3830](https://github.com/everruns/everruns/pull/3830),
[#3977](https://github.com/everruns/everruns/pull/3977).
