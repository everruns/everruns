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

Slack, webhooks, GitHub triggers and OAuth callbacks need an address the
internet can reach. `everruns docker up` now [gives a local instance one](/getting-started/run-locally/#give-it-a-public-address):

```bash
TS_AUTHKEY=tskey-auth-... everruns docker up --tunnel tailscale
```

The tunnel runs as one more container in the stack, so nothing is installed on
your computer. `up` waits for the address, passes it to the server as
`PUBLIC_APP_URL`, checks that it answers and prints it.

- **`tailscale`**: a stable `*.ts.net` address through Tailscale Funnel. Recommended.
- **`cloudflare`**: a named tunnel on your hostname, or a quick
  `*.trycloudflare.com` address that changes on restart.
- **`ngrok`**: your ngrok static domain.
- **`--public-url`** alone, when a reverse proxy already points at the machine.

`up` refuses a public address with `--auth none`. The Compose file moved to
`infra/docker-compose/`, with a profile per tunnel.
