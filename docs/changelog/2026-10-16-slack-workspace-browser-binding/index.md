---
title: Slack workspace consent stays with its initiating browser
description: Slack manager workspace connections now require the initiating browser and a single-use transaction, with independent tabs and safe consent retries.
date: 2026-10-16
kind: fix
significance: minor
products: [platform, cloud]
release: "0.49.0"
surfaces: [api]
links:
  docs: [/how-to/publish-to-slack/]
---

Connecting a workspace through a configured Slack manager app now binds consent
to the browser that started it. Each connection attempt expires after thirty
minutes and can finish only once, including when two callbacks arrive together.
Separate tabs keep their own attempts.

Complete Slack consent in the same browser. If consent expires, is cancelled,
or fails during exchange, start a new connection attempt. A callback opened in
another browser cannot connect the workspace or invalidate the original attempt.
See [Publish an Agent to Slack](/how-to/publish-to-slack/) for setup guidance.
