---
title: Session trace explains failed turns and model calls
description: The session trace names why a turn failed, shows model call retries, and its inspector now shows a failed model call's full error with a link to the provider.
date: 2026-11-01
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  prs: []
---

Selecting a failed model call in the session trace now opens its error in the
inspector. Before, the Error section showed an empty `{}`. The section leads with
the provider's own message, links the provider account that served the call, and
names who reported the error and its HTTP status, with the raw error below.

In the trace list a failed step keeps its icon and shows a Failed badge and the
first three lines of its error; "Show full error" opens the inspector. The step
line runs through the whole turn and ends on its footer, so a failed step and the
"Turn failed" summary read as part of the turn. A turn that fails because a step
failed now counts one error instead of two.

A failed turn's header names the kind of failure, such as Out of credits, Rate
limited or Provider down, and a model call that needed retries shows how many it
made and how long it waited. Turns recorded before this change show the error
text without the label.
