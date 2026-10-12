---
title: "GitHub tools act as one chosen identity, with narrowed tokens"
description: "The GitHub capability's acts_as setting picks the agent, the person, or the person when connected; every call gets a token limited to one repository and the permissions that tool needs."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/github/]
  prs: []
---

The GitHub capability's tools now act on GitHub as exactly one identity, set by
the new `acts_as` setting: `agent` (the default), `person`, or
`person_or_agent`. Before, a run started by a person who had connected GitHub
used that person's connection, so a review bot could post as the Everruns app
instead of as itself.

```json
{ "ref": "github", "config": { "acts_as": "person" } }
```

Every call also gets a short-lived token limited to the repository it names and
the permissions that tool needs, so reading a diff no longer holds write access.
