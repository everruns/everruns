---
title: Run Everruns on your machine
description: Start the self-hosted Platform on your computer with one command, everruns docker up, which asks a few questions, starts it in Docker and signs the CLI in.
sidebar:
  label: On your machine
appliesTo: [platform]
---

`everruns docker up` starts the self-hosted Platform on your computer: the API,
a worker, PostgreSQL and the UI, in Docker. It asks a few questions the first
time, generates the secrets, waits until the server is healthy and signs the
CLI in to it. Running it again is safe: it starts what is stopped, repairs what
is missing and leaves everything else alone.

## Prerequisites

- Docker Engine 20.10+ with Compose 2.20+ (Docker Desktop, OrbStack or Colima
  on macOS), with about 2 GB of memory for Docker
- The `everruns` CLI ([install](/features/cli/#install))

## Start it

```bash
everruns docker up
```

```text
✓ Docker: engine 27.3.1
✓ Docker Compose: 2.29.7
Who signs in?: Just me, with a password
Admin email: you@example.com
LLM provider: OpenAI (use OPENAI_API_KEY from your shell)

Everruns 0.48.0 is running at http://localhost:9300
  Sign in: you@example.com / pNhN...  (shown once; also in ~/.local/share/everruns/docker/default/.env)
  CLI: signed in as you@example.com (profile 'local'), try `everruns agents list`
```

The first run pulls the images, so it takes a minute or two. Later runs take a
second or two when nothing changed.

`everruns docker doctor` runs the same checks on their own. When Docker is
missing, stopped, too old, or your user may not use it, each check says what to
do about it.

## The questions

Every question has a flag, and `--yes` accepts the defaults, so scripts and CI
run without prompts:

```bash
everruns docker up --yes --auth single --admin-email ci@example.com --provider openai
```

| Question | Flag | Default |
|---|---|---|
| Who signs in | `--auth single`, `none` or `accounts` | `single` |
| Admin email | `--admin-email` | your `git config user.email` |
| LLM provider | `--provider openai`, `anthropic`, `gemini` or `skip` | the provider whose key is in your shell |
| Port, only when 9300 is taken | `--port` | the next free port |

**Who signs in:**

- `single`: one admin account with a generated password. Good for your own
  machine.
- `none`: no sign-in, every request is admin. The port then listens on
  127.0.0.1 only, and the server runs as a dev deployment, which also turns on
  experimental features. Use it for throwaway experiments.
- `accounts`: people sign up and sign in with a password. The CLI creates your
  account first. Each organization adds its own provider keys in the UI, so
  `--provider` does not apply.

The provider key is read from `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` or
`GEMINI_API_KEY`, or typed at a hidden prompt, never from a command-line
argument. It becomes the default organization's provider key.

Change an answer by passing its flag again, for example
`everruns docker up --auth none`. Only the containers whose settings changed
restart. Moving away from `none` revokes the tokens made without sign-in, and
`up` signs the CLI in again.

## What it keeps

Settings live in one folder per instance: `~/.local/share/everruns/docker/default/`
on Linux, `~/Library/Application Support/everruns/docker/default/` on macOS.

- `.env` holds the answers, the admin password and the generated secrets,
  readable only by you. **Keep it**: it holds the encryption key for the
  provider keys and secrets stored in the database, and without it they cannot
  be read.
- `compose.yaml` is the [Docker Compose](/getting-started/docker-compose/)
  stack without its optional profiles.

- `backups/` holds the database backups `upgrade` takes.

The database lives in a Docker volume. `up` never regenerates a secret.

## Upgrade

The images match the CLI's version when the instance is set up, and `up` never
moves them. After you upgrade the CLI, `up` tells you when the instance runs an
older version, and `upgrade` moves it:

```bash
everruns docker upgrade                      # to this CLI's version
everruns docker upgrade --image-tag v0.49.0  # or a tag you name
```

The new server runs the database migrations when it starts, and they only go
forward, so `upgrade` first saves a database backup in the instance's
`backups/` folder (`--no-backup` skips it). Moving to an older tag asks first:
it needs the backup taken before the newer version to work. To restore one,
copy it into the `postgres` container and run `pg_restore --clean` against the
`everruns` database there.

## Start over

```bash
everruns docker reset
```

`reset` deletes the instance: its containers, the database, the settings folder
with the encryption key and backups, and the CLI profile. It asks you to type
the instance name first; `--yes` skips that for scripts.

## Day to day

```bash
everruns docker status         # URL, health, version, profile and containers
everruns docker logs server    # follow one service's logs
everruns docker open           # open the UI
everruns docker stop           # stop it; data is kept
```

The CLI profile is `local`. It becomes the current profile, after asking when
another one is in use. Use `everruns --profile local ...` to reach the local
instance while another profile stays current.

## More than one instance

`--name` keeps a separate instance with its own settings, data, port and CLI
profile:

```bash
everruns docker up --name demo   # profile 'local-demo', next free port
```

## Give it a public address

Slack, webhook channels, GitHub triggers and OAuth callbacks need an address
the internet can reach. `--tunnel` adds one, running as one more container in
the stack, so it starts and stops with everything else and nothing is
installed on your computer:

```bash
TS_AUTHKEY=tskey-auth-... everruns docker up --tunnel tailscale
```

| Tunnel | Address | You need |
|---|---|---|
| `tailscale` (recommended) | Stable, `https://everruns.<tailnet>.ts.net` | A free Tailscale account and an [auth key](https://login.tailscale.com/admin/settings/keys). Allow Funnel in the [tailnet policy](https://tailscale.com/kb/1223/funnel) and turn on HTTPS certificates. |
| `cloudflare --public-url https://agents.example.com` | Stable, your hostname | A domain on Cloudflare and a tunnel token (`CLOUDFLARE_TUNNEL_TOKEN`). Point the tunnel's public hostname at `http://caddy:9300`. |
| `cloudflare` | Random `*.trycloudflare.com`, **changes on every restart** | Nothing. For a quick try only: Slack apps and OAuth callbacks stop working when it changes. |
| `ngrok --public-url https://<name>.ngrok-free.app` | Your ngrok static domain | An ngrok account and `NGROK_AUTHTOKEN`. The free plan shows browsers a warning page first. |

The token is read from the environment, or asked for once, and kept in the
instance's `.env`. `up` waits for the address, passes it to the server as
`PUBLIC_APP_URL`, and checks that it answers. `everruns docker status` shows it.

Already have a reverse proxy or a domain pointing at this machine? Pass the
address alone:

```bash
everruns docker up --public-url https://agents.example.com
```

`--tunnel none` turns a tunnel off and `--public-url ''` removes the address.
A public address needs sign-in: `up` refuses `--tunnel` and `--public-url`
together with `--auth none`, where anyone who reaches the instance is admin.

## Use another container CLI

`EVERRUNS_DOCKER=podman everruns docker up` runs Podman's Docker-compatible
CLI instead of `docker`. Docker is the tested path.
