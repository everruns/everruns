---
title: "Browser agents see their screenshots and keep their page"
description: "Browser screenshots now reach the model, Browserless tools no longer race on one page, and key presses report where focus landed."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  prs: [everruns/everruns#4587]
---

Screenshots from the `browser` and `browserless_screenshot` tools now reach the
model. Before, any image over 64 KiB (almost every screenshot) was silently
dropped; images now have their own 5 MiB budget, and a dropped image is named in
the tool result.

Browserless tools and the `browser` tool share one browser per session, and now
run one at a time instead of racing on the same page. `browserless_content`,
`browserless_scrape` and `browserless_screenshot` read in a tab of their own, so
they no longer move the page the agent is working in.

A `browser` key press now reports where focus landed (`Pressed Tab. Focus is on
link "Vision".`), and `read_page` marks the focused element, which makes
keyboard and accessibility testing practical.

Connecting a connector that is turned off for the organization now says so, and
where an admin turns it on, instead of reporting an unknown connector.
