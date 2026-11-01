---
title: Session trace explains failed turns and model calls
description: The session trace names why a turn failed, shows model call retries, and its inspector now shows a failed model call's full error with a link to the provider.
date: 2026-11-01
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  prs: [everruns/everruns#5133]
---

Selecting a failed model call in the session trace now opens its error in the
inspector, which used to show an empty `{}`. The inspector leads with the
provider's message, links the provider account that served the call, and names
the HTTP status, with the raw error below.

A failed step keeps its icon, shows a Failed badge and the first lines of its
error, and stays on the turn's step line. A failed turn's header names the kind
of failure, such as Out of credits, Rate limited or Provider down. Model calls
that needed retries show how many they made. A turn that fails because a step
failed counts one error instead of two.
