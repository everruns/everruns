---
title: Give a local Everruns a public address with one flag
description: everruns docker up --tunnel tailscale, cloudflare or ngrok runs a tunnel inside the stack and sets PUBLIC_APP_URL, so Slack and webhooks reach your laptop.
date: 2026-11-08
kind: feature
significance: minor
products: [platform]
surfaces: [cli]
links:
  docs: [/getting-started/run-locally/#give-it-a-public-address]
---

Slack, webhook channels, GitHub triggers and OAuth callbacks need an address the
internet can reach. `everruns docker up` now gives a local instance one:

```bash
TS_AUTHKEY=tskey-auth-... everruns docker up --tunnel tailscale
```

The tunnel runs as one more container in the Compose stack, so it starts and
stops with the rest and nothing is installed on your computer. `up` waits for
the address, passes it to the server as `PUBLIC_APP_URL`, checks that it answers,
and prints it; `everruns docker status` shows it too.

- **`tailscale`**: a stable `*.ts.net` address through Tailscale Funnel, free, no
  domain needed. The one the docs recommend.
- **`cloudflare`**: a named tunnel on your own hostname with `--public-url`, or a
  quick `*.trycloudflare.com` address for a one-off try (it changes on restart).
- **`ngrok`**: your ngrok static domain.
- **`--public-url https://agents.example.com`** alone, when a reverse proxy or DNS
  already points at the machine.

A public address needs sign-in: `up` refuses it together with `--auth none`.
The Compose file moved to `infra/docker-compose/docker-compose.yaml`, now passes
`PUBLIC_APP_URL` to the server, and has a profile per tunnel for people who run
Compose by hand. See [Give it a public address](/getting-started/run-locally/#give-it-a-public-address).
