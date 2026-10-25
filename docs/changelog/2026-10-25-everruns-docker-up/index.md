---
title: Run Everruns on your machine with one command
description: everruns docker up asks a few questions, starts the self-hosted Platform in Docker and signs the CLI in; running it again is safe.
date: 2026-10-25
kind: feature
significance: major
products: [platform]
surfaces: [cli]
links:
  docs: [/getting-started/run-locally/]
  prs: [everruns/everruns#4895, everruns/everruns#4904, everruns/everruns#4906]
---

`everruns docker up` starts the self-hosted Platform on your computer. It checks
Docker, asks who signs in (just you, nobody, or accounts), your admin email and
an LLM provider, then generates the secrets, waits until the server is healthy
and signs the CLI in as profile `local`. Running it again starts what is
stopped and leaves the rest alone. `everruns docker upgrade` backs up the
database and moves the instance to the CLI's version; `everruns docker reset`
deletes it after you type its name.

```bash
everruns docker up --yes --provider openai   # no prompts, key from OPENAI_API_KEY
everruns agents list
```

The example compose file now starts one worker by default; the
[Docker Compose guide](/getting-started/docker-compose/) turns on the `scale`
and `metrics` profiles for the previous shape. See
[Run Everruns on your machine](/getting-started/run-locally/).
