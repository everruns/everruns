---
title: "Web search no longer fails on some result snippets"
description: "A Brave or free search whose result text had an ampersand a few characters before a dash, curly quote or accented letter no longer fails."
date: 2026-10-13
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [api]
release: "0.48.0"
links:
  docs: [/capabilities/web-search/]
  prs: [everruns/everruns#4678]
---

A [Web Search](/capabilities/web-search/) call through Brave Search or free search could fail
outright when a result's title or snippet held an `&` followed about ten characters later by a
character outside plain ASCII, such as a dash, a curly quote or an accented letter. Those results
are now read like any other, and the search returns them.
