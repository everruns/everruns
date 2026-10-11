---
title: Everruns Cloud verify-email screen sends a new link again
description: The email verification step on Everruns Cloud now offers Sign in again, which sends a fresh link, replacing a resend action that never sent any mail.
date: 2026-10-10
kind: fix
significance: minor
products: [cloud]
surfaces: [ui]
links:
  docs: [/getting-started/cloud/]
---

On [Everruns Cloud](https://app.everruns.com), the screen that asks you to
verify your email had a resend action that reported success but sent nothing,
so an expired verification link left you stuck.

The screen now offers **Sign in again**. Signing in again at Everruns ID sends a
fresh verification link if the old one expired, and you continue to onboarding
once the address is verified.

See [Everruns Cloud quickstart](/getting-started/cloud/).
