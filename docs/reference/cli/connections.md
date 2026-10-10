---
title: everruns connections
description: "Your LLM provider API keys. CLI reference for everruns connections."
sidebar:
  label: connections
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Your LLM provider API keys.

| Command | What it does |
|---|---|
| [`connections set`](#connections-set) | Set an API key connection for a provider. |
| [`connections list`](#connections-list) | List connected providers. |
| [`connections remove`](#connections-remove) | Remove a connection. |

## connections set

Set an API key connection for a provider.

```bash
everruns connections set [OPTIONS] <PROVIDER>
```

| Flag | Description |
|---|---|
| `<PROVIDER>` | Required. Provider name (e.g. daytona, brave_search, browserless, deno, sprites) |
| `--api-key-stdin` | Read provider API key from stdin. |


## connections list

List connected providers.

```bash
everruns connections list
```


## connections remove

Remove a connection.

```bash
everruns connections remove <PROVIDER>
```

| Flag | Description |
|---|---|
| `<PROVIDER>` | Required. Provider name (e.g. daytona, brave_search) |

