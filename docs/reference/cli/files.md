---
title: everruns files
description: "Sync files between a local folder and a session. CLI reference for everruns files."
sidebar:
  label: files
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Sync files between a local folder and a session.

| Command | What it does |
|---|---|
| [`files sync`](#files-sync) | Bidirectional live sync between local directory and session workspace. |
| [`files push`](#files-push) | One-shot push local files to session workspace. |
| [`files pull`](#files-pull) | One-shot pull session workspace files to local directory. |
| [`files ls`](#files-ls) | List files in session workspace. |

## files sync

Bidirectional live sync between local directory and session workspace.

```bash
everruns files sync [OPTIONS] --session <SESSION> [LOCAL_DIR]
```

| Flag | Description |
|---|---|
| `-s`, `--session <SESSION>` | Required. Session ID (e.g. ses_xxx) |
| `<LOCAL_DIR>` | Local directory to sync (default: current directory) |
| `--interval <INTERVAL>` | Remote poll interval in seconds. |
| `--conflict <CONFLICT>` | Conflict strategy. One of `last-write-wins`, `local-wins`, `remote-wins`. |
| `--exclude <EXCLUDE>` | Additional exclude patterns (repeatable) Repeatable. |
| `--no-gitignore` | Don't read .gitignore. |
| `--dry-run` | Show what would sync without making changes. |
| `--delete` | Delete files on one side when deleted on the other. |
| `-v`, `--verbose` | Show every file operation. |


## files push

One-shot push local files to session workspace.

```bash
everruns files push [OPTIONS] --session <SESSION> [LOCAL_DIR]
```

| Flag | Description |
|---|---|
| `-s`, `--session <SESSION>` | Required. Session ID (e.g. ses_xxx) |
| `<LOCAL_DIR>` | Local directory to push from (default: current directory) |
| `--delete` | Delete remote files not present locally. |
| `--dry-run` | Show what would be pushed. |


## files pull

One-shot pull session workspace files to local directory.

```bash
everruns files pull [OPTIONS] --session <SESSION> [LOCAL_DIR]
```

| Flag | Description |
|---|---|
| `-s`, `--session <SESSION>` | Required. Session ID (e.g. ses_xxx) |
| `<LOCAL_DIR>` | Local directory to pull into (default: current directory) |
| `--delete` | Delete local files not present remotely. |
| `--dry-run` | Show what would be pulled. |


## files ls

List files in session workspace.

```bash
everruns files ls [OPTIONS] --session <SESSION> [PATH]
```

| Flag | Description |
|---|---|
| `-s`, `--session <SESSION>` | Required. Session ID (e.g. ses_xxx) |
| `<PATH>` | Remote path to list (default: root) |
| `-r`, `--recursive` | List recursively. |
| `-l`, `--long` | Show size and dates. |

