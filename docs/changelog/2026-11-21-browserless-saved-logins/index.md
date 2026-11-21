---
title: Browser agents stay signed in between sessions
description: Save a Browserless sign-in by name, or make one yourself in the dashboard, and the next session opens already logged in without the password.
date: 2026-11-21
kind: feature
significance: notable
products: [framework, platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/browserless/]
---

An agent that checks a dashboard every morning used to sign in every morning:
type the password, wait for the redirect, hope there is no MFA prompt today.
Now it signs in once and keeps the result. `browserless_save_login` stores the
browser's cookies and the page's local storage as a named login in your
[Browserless](/capabilities/browserless/) account, and the next session opens
straight into the signed-in page:

```json
{
  "url": "https://grafana.example.com/d/daily",
  "login": "grafana-ops"
}
```

A login can also come from you. Sign in through the Browserless dashboard's
remote browser, MFA and CAPTCHA included, give it a name, and the agent uses it
without ever seeing the password. `browserless_list_logins` shows what is saved,
with counts but never cookie values.

The login is restored into the same guarded browser every Browserless tool
uses, so the session's allowed sites and the private-network block still
apply.
