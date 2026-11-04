---
title: Request signing now verifies with Cloudflare Web Bot Auth
description: Agent web requests are signed in the format Cloudflare verifies, and the key directory is signed so a deployment can register as a verified bot.
date: 2026-11-04
kind: fix
significance: notable
products: [platform]
surfaces: [operations]
links:
  docs: [/sre/request-signing/]
---

Request signing used a format that Web Bot Auth verifiers reject, so signed
agent requests were never recognized. Requests from `web_fetch` and `curl` in
the shell are now signed by the egress layer to the current IETF
[Web Bot Auth](/sre/request-signing/) draft. They cover the method, host and
path, and name your deployment's origin in `Signature-Agent`. The key
directory at `/.well-known/http-message-signatures-directory` is signed per
key, which Cloudflare requires before you can register it as a verified bot.

Set the seed and the origin that serves the directory on the server and every
worker:

```bash
BOT_AUTH_SIGNING_KEY_SEED="<base64url 32-byte seed>"
BOT_AUTH_SIGNATURE_AGENT_URL="https://agents.example.com"
```

`BOT_AUTH_AGENT_FQDN` is gone. `BOT_AUTH_PUBLISHED_KEY_SEEDS` lists extra keys
for rotation.
