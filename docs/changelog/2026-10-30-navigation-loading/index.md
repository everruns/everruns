---
title: Page navigation keeps content visible while loading
description: Navigating between Everruns pages keeps the current content visible until the destination is ready, avoiding flashes of generic page placeholders.
date: 2026-10-30
kind: fix
significance: minor
products: [platform, cloud]
release: "0.50.1"
surfaces: [ui]
---

Navigating between pages now keeps the current page visible until the destination
is ready, instead of briefly replacing titles, breadcrumbs and controls with
generic placeholders. A loading indicator on the clicked sidebar link shows
that navigation is in progress.

For example, switching from Sandboxes to Harnesses keeps the Sandboxes page
visible until Harnesses can render. Session views continue to load their content
inside the persistent session header.
