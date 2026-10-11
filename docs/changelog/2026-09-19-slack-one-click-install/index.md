---
title: Install an Everruns agent in Slack with one click
description: "Connecting an agent to Slack no longer needs credentials copied from api.slack.com: Everruns creates the Slack app and takes its credentials from OAuth."
date: 2026-09-19
kind: feature
significance: notable
products: [cloud, platform]
surfaces: [ui, operations]
release: "0.30.0"
links:
  docs: [/how-to/publish-to-slack/, /capabilities/slack/]
  prs: [everruns/everruns#3724, everruns/everruns#3721]
visual: "Slack endpoint editor on an agent's Integrations tab showing the Connect to Slack button (/agents/<id>, Integrations); crop to the endpoint form"
---

Connecting an agent to Slack used to mean copying four values out of
api.slack.com. Now you name the Slack endpoint, save it and choose **Connect to
Slack**: Everruns creates the app from the manifest it generates, Slack shows one
consent screen, and the signing secret, bot token and workspace ID arrive on
their own.

One-click install needs a Slack app configuration token on the deployment, which
Everruns Cloud has. Without one, the button is hidden and the manual fields stay
available under **Configure manually**.

See [Publish to Slack](/how-to/publish-to-slack/) and [Slack](/capabilities/slack/). [#3724](https://github.com/everruns/everruns/pull/3724)
