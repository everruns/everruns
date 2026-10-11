---
title: The Everruns UI supports dark mode with a theme setting
description: The Everruns web UI can now be shown in light, dark or system theme, chosen from the user menu and remembered without a flash on navigation.
date: 2026-09-16
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.28.0"
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#3637, everruns/everruns#3639]
visual: "Agents list in dark theme with the user menu Theme submenu open (/agents); crop to sidebar menu and part of the page"
---

The Everruns UI has a dark theme. Open the user menu in the sidebar
and pick **Theme**: light, dark or system. The choice is stored in a cookie, so
pages render in the right theme on the server and navigation does not flash.
Scrollbars and native controls follow the theme too.

Before this change, a machine set to dark mode got a partly dark interface; the
whole UI now switches together, with semantic status colors that work in both
themes.

See [UI](/features/ui/). [#3637](https://github.com/everruns/everruns/pull/3637), [#3639](https://github.com/everruns/everruns/pull/3639)
