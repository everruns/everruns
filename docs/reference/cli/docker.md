---
title: everruns docker
description: "Run Everruns on this machine with Docker. CLI reference for everruns docker: usage, flags and examples."
sidebar:
  label: docker
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Run Everruns on this machine with Docker.

| Command | What it does |
|---|---|
| [`docker up`](#docker-up) | Start Everruns on this machine, setting it up on first run. |
| [`docker status`](#docker-status) | Show whether the local instance runs, where, and which CLI profile points at it. |
| [`docker stop`](#docker-stop) | Stop the containers. |
| [`docker logs`](#docker-logs) | Show the containers' logs. |
| [`docker open`](#docker-open) | Open the local UI in the browser. |
| [`docker doctor`](#docker-doctor) | Check that Docker is installed, running and new enough. |
| [`docker upgrade`](#docker-upgrade) | Move the instance to this CLI's version (or --image-tag), backing up the database first. |
| [`docker reset`](#docker-reset) | Delete the instance: containers, database and settings. |

## docker up

Start Everruns on this machine, setting it up on first run. Safe to run again.

```bash
everruns docker up [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |
| `--auth <AUTH>` | Who signs in. One of `single`, `none`, `accounts`. |
| `--admin-email <ADMIN_EMAIL>` | Admin email, used to sign in to the UI. |
| `--provider <PROVIDER>` | LLM provider for the server's default key. The key comes from OPENAI_API_KEY, ANTHROPIC_API_KEY or GEMINI_API_KEY, or a prompt. One of `openai`, `anthropic`, `gemini`, `skip`. |
| `--port <PORT>` | Port for the UI and API on this machine (default 9300) |
| `--image-tag <IMAGE_TAG>` | Image tag to run, such as v0.48.0 or latest (default: this CLI's version) |
| `-y`, `--yes` | Accept the default for every question. |
| `--no-open` | Do not open the browser. |


## docker status

Show whether the local instance runs, where, and which CLI profile points at it.

```bash
everruns docker status [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |


## docker stop

Stop the containers. Data and settings are kept.

```bash
everruns docker stop [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |


## docker logs

Show the containers' logs.

```bash
everruns docker logs [OPTIONS] [SERVICE]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |
| `<SERVICE>` | Only this service (server, worker-1, ui, caddy, postgres, ...) |
| `--no-follow` | Print what is there and exit instead of following. |


## docker open

Open the local UI in the browser.

```bash
everruns docker open [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |


## docker doctor

Check that Docker is installed, running and new enough.

```bash
everruns docker doctor [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |


## docker upgrade

Move the instance to this CLI's version (or --image-tag), backing up the database first.

```bash
everruns docker upgrade [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |
| `--image-tag <IMAGE_TAG>` | Image tag to move to, such as v0.49.0 or latest (default: this CLI's version) |
| `--no-backup` | Skip the database backup taken before the upgrade. |
| `-y`, `--yes` | Do not ask, also when the move is to an older version. |


## docker reset

Delete the instance: containers, database and settings. Asks first.

```bash
everruns docker reset [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Instance name, for more than one local Everruns. |
| `-y`, `--yes` | Do not ask for the instance name (for scripts) |

