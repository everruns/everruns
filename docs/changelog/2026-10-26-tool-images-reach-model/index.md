---
title: Agents see the screenshots and images their tools return
description: Images from tool results, such as computer-use screenshots, reach the model as images again instead of as truncated base64 text.
date: 2026-10-26
kind: fix
significance: minor
products: [platform, cloud]
release: "0.49.0"
surfaces: [api]
links:
  docs: [/capabilities/computer-use/]
  prs: [everruns/everruns#4923]
---

When a tool returned an image, such as a `computer` screenshot, a browser capture or an image
read from a file, the worker received it as base64 inside the tool result's text. The text was then
truncated, so the model never saw the picture: computer-use agents reported that screenshots
failed and fell back to reading the screen through shell tools. Tool results now keep their images
as image parts on every path to the model. See [Computer Use](/capabilities/computer-use/).
