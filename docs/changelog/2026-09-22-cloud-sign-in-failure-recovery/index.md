---
title: Everruns Cloud sign-in failures return to the login page
description: A failed Everruns Cloud sign-in now lands on the login page with the reason and a way to try again, instead of a raw JSON error on the callback URL.
date: 2026-09-22
kind: fix
significance: minor
products: [cloud]
surfaces: [ui]
links:
  docs: [/getting-started/cloud/]
---

A sign-in to [Everruns Cloud](https://app.everruns.com) that did not complete
used to leave you on the callback URL looking at a JSON error such as
`{"error":"Login temporarily unavailable"}`, with no way forward.

Now it returns to the login page and says whether sign-in was cancelled, the
10-minute sign-in window expired, or Everruns ID could not be reached, with a
**Try signing in again** action. A connection failure while exchanging the login
code is retried before it fails, and a malformed token response is reported as
unavailable rather than as an expired link.

See [Everruns Cloud quickstart](/getting-started/cloud/).
