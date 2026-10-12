---
title: everruns notifications
description: "Your in-app notifications. CLI reference for everruns notifications."
sidebar:
  label: notifications
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Your in-app notifications.

| Command | What it does |
|---|---|
| [`notifications list`](#notifications-list) | List notifications for the current user. |
| [`notifications view mark`](#notifications-view-mark) | Mark a notification as viewed. |

## notifications list

List notifications for the current user.

```bash
everruns notifications list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Maximum number of items returned in this page. |

Example:

```bash
# Catch up on notifications addressed to you
everruns notifications list --limit 20
```

## notifications view mark

Mark a notification as viewed.

```bash
everruns notifications view mark [OPTIONS] [NOTIFICATION_ID]
```

| Flag | Description |
|---|---|
| `--notification-id <NOTIFICATION_ID>` | Prefixed public identifier of the notification to mark as viewed. |

Example:

```bash
# Clear a notification once you have read it
everruns notifications view mark notification_01h9
```
