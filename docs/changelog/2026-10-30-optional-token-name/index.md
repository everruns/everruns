---
title: Personal access tokens suggest today's date
description: The create dialog lists the name last and marks it optional. It is filled in as Personal token plus today's date, and a blank name keeps that suggestion.
date: 2026-10-30
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/ui/]
visual: "Create personal access token dialog with Access and Expiration above an optional Name filled as Personal token plus today's date (/settings/personal-access-tokens)"
---

The create personal access token dialog no longer asks for a name first. Name is the last field, marked optional, and starts as Personal token plus today's date:

```text
Personal token (Oct 30, 2026)
```

Replace that suggestion, or clear the field. A blank name keeps it, so the token list still shows a name. Access and expiration are unchanged: full access and 90 days unless you pick something else. See [Management UI](/features/ui/).
