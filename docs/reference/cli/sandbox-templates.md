---
title: everruns sandbox-templates
description: "Reusable sandbox configurations and their provider snapshots. CLI reference for everruns sandbox-templates: usage, flags and examples."
sidebar:
  label: sandbox-templates
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Reusable sandbox configurations and their provider snapshots.

| Command | What it does |
|---|---|
| [`sandbox-templates builds apply`](#sandbox-templates-builds-apply) | Atomically apply a ready snapshot to its unchanged source template. |
| [`sandbox-templates builds cancel`](#sandbox-templates-builds-cancel) | Cancel queued or running build work and clean up its disposable resources. |
| [`sandbox-templates builds discard`](#sandbox-templates-builds-discard) | Remove an unused build artifact. |
| [`sandbox-templates builds get`](#sandbox-templates-builds-get) | Inspect a build attempt in the authenticated organization. |
| [`sandbox-templates builds list`](#sandbox-templates-builds-list) | List twenty build attempts at an optional offset, bounded logs, and captured software notes. |
| [`sandbox-templates builds start`](#sandbox-templates-builds-start) | Run an install script in fresh isolated compute and bake a reusable provider snapshot. |

## sandbox-templates builds apply

Atomically apply a ready snapshot to its unchanged source template. Existing Agent and Session pins do not move.

```bash
everruns sandbox-templates builds apply [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed identifier with 'sbxbuild' prefix. |

Example:

```bash
# Apply a Sandbox Template build
everruns sandbox-templates builds apply --id sbxbuild_01933b5a000070008000000000000001
```

## sandbox-templates builds cancel

Cancel queued or running build work and clean up its disposable resources.

```bash
everruns sandbox-templates builds cancel [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed identifier with 'sbxbuild' prefix. |

Example:

```bash
# Cancel a Sandbox Template build
everruns sandbox-templates builds cancel --id sbxbuild_01933b5a000070008000000000000001
```

## sandbox-templates builds discard

Remove an unused build artifact. Applied artifacts remain pinned and cannot be discarded.

```bash
everruns sandbox-templates builds discard [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed identifier with 'sbxbuild' prefix. |

Example:

```bash
# Discard a Sandbox Template build
everruns sandbox-templates builds discard --id sbxbuild_01933b5a000070008000000000000001
```

## sandbox-templates builds get

Inspect a build attempt in the authenticated organization.

```bash
everruns sandbox-templates builds get [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed identifier with 'sbxbuild' prefix. |

Example:

```bash
# Get a Sandbox Template build
everruns sandbox-templates builds get --id sbxbuild_01933b5a000070008000000000000001
```

## sandbox-templates builds list

List twenty build attempts at an optional offset, bounded logs, and captured software notes.

```bash
everruns sandbox-templates builds list [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed identifier with 'sbxtpl' prefix. |
| `--offset <OFFSET>` |  |

Example:

```bash
# List a Sandbox Template build
everruns sandbox-templates builds list --id sbxtpl_01933b5a000070008000000000000001
```

## sandbox-templates builds start

Run an install script in fresh isolated compute and bake a reusable provider snapshot. Does not revise the template.

```bash
everruns sandbox-templates builds start [OPTIONS] --id <id> --install-script <install_script> --source-revision-id <source_revision_id>
```

| Flag | Description |
|---|---|
| `--build-network <BUILD_NETWORK>` |  |
| `--id <ID>` | Required. Prefixed identifier with 'sbxtpl' prefix. |
| `--install-script <INSTALL_SCRIPT>` | Required. |
| `--metadata-path <METADATA_PATH>` | Optional bounded, author-supplied Markdown captured from the builder. |
| `--source-revision-id <SOURCE_REVISION_ID>` | Required. Source revision to guard against concurrent template edits. |
| `--timeout-seconds <TIMEOUT_SECONDS>` |  |
| `--verification-command <VERIFICATION_COMMAND>` |  |

Example:

```bash
# Start a Sandbox Template build
everruns sandbox-templates builds start --id sbxtpl_01933b5a000070008000000000000001 --source-revision-id sbxtplrev_01933b5a000070008000000000000001 --install-script 'echo ready'
```
