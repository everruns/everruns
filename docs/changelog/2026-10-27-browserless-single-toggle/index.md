---
title: Browserless settings collapse into one toggle
description: The three Browserless adoption toggles become one Browserless flag; agents pick computer use or browser use via capabilities.
date: 2026-10-27
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
---
The Browserless section of adoption settings showed three separate toggles:
Browserless, Browserless browser use, and Browserless computer use. They are
now a single Browserless toggle. Turning it on connects your Browserless
account and unlocks the `browser_use` and `computer_use` capabilities, which
agents pick individually with their own options.
