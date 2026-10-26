---
title: Raw event previews stay small for large tool results
description: The Events table bounds JSON previews while keeping complete events available through Copy.
date: 2026-10-26
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: /features/events/
---

The session **Events** table now shortens oversized raw JSON previews before
rendering them. Screenshot data, long strings and large nested collections no
longer become unrestricted text blocks inside each visible row. Ordinary small
events remain fully readable, and a notice marks every shortened preview.

Use **Copy** to retrieve the complete original event. This changes the browser
preview only: stored events, API responses and event streaming are unchanged.
Read about [inspecting raw events](/features/events/#inspecting-raw-events-in-the-ui).
