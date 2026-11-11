---
title: "Browser sessions are bound to the device in Chrome"
description: "Sign-ins now support Device Bound Session Credentials, so a session cookie copied off a Chrome machine stops working within minutes."
date: 2026-11-10
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, operations]
links:
  docs: [/sre/runbooks/authentication/]
---

Signing in from Chrome now binds the session to a key in the device's secure
hardware, using
[Device Bound Session Credentials](https://w3c.github.io/webappsec-dbsc/).
The browser drops the 30-day refresh cookie and renews its short-lived access
cookie itself, signing each renewal with that key. A cookie lifted by
infostealer malware or from a saved HAR file no longer outlives its first few
minutes, and the refresh token it came with is refused.

The sign-in response offers the binding, and renewals carry a signed proof:

```http
Secure-Session-Registration: (ES256 RS256);path="dbsc/start";challenge="..."

POST /api/v1/auth/dbsc/refresh
Sec-Secure-Session-Id: "..."
Secure-Session-Response: "<proof signed by the device key>"
```

Nothing to configure. Browsers without DBSC sign in exactly as before.
Everruns ID applies the same binding to its single sign-on cookie, and its
sessions page marks bound sign-ins "Device-bound". Proxy requirements are in
[Authentication](/sre/runbooks/authentication/#device-bound-sessions).
