---
title: everruns k8s
description: "Run Everruns in a Kubernetes cluster with the Helm chart. CLI reference for everruns k8s: usage, flags and examples."
sidebar:
  label: k8s
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Run Everruns in a Kubernetes cluster with the Helm chart.

| Command | What it does |
|---|---|
| [`k8s up`](#k8s-up) | Install Everruns into the current Kubernetes cluster, or repair it. |
| [`k8s status`](#k8s-status) | Show the install: where it is, its pods, and which CLI profile points at it. |
| [`k8s logs`](#k8s-logs) | Show the pods' logs. |
| [`k8s open`](#k8s-open) | Open the UI in the browser (through a port-forward when there is no public URL) |
| [`k8s port-forward`](#k8s-port-forward) | Keep a port-forward open so the CLI and browser reach an install without a public URL. |
| [`k8s doctor`](#k8s-doctor) | Check the tools and the cluster, and run the system doctor on the server. |
| [`k8s password`](#k8s-password) | Print the admin password the chart generated. |
| [`k8s upgrade`](#k8s-upgrade) | Move the install to this CLI's chart version (or --chart-version), backing up a bundled database first. |
| [`k8s uninstall`](#k8s-uninstall) | Remove the workloads. |
| [`k8s reset`](#k8s-reset) | Delete the install: workloads, generated secrets, volumes and the CLI profile. |

## k8s up

Install Everruns into the current Kubernetes cluster, or repair it. Safe to run again.

```bash
everruns k8s up [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |
| `--context <CONTEXT>` | Kubernetes context (default: kubectl's current context) |
| `--namespace <NAMESPACE>` | Namespace, created when missing (default: everruns) |
| `--auth <AUTH>` | Who signs in. One of `single`, `none`, `accounts`. |
| `--admin-email <ADMIN_EMAIL>` | Admin email, used to sign in to the UI. |
| `--provider <PROVIDER>` | LLM provider for the server's default key. The key comes from OPENAI_API_KEY, ANTHROPIC_API_KEY, GEMINI_API_KEY or OPENROUTER_API_KEY, or a prompt. One of `openai`, `anthropic`, `gemini`, `openrouter`, `skip`. |
| `--database-url <DATABASE_URL>` | Your PostgreSQL 15+ URL (postgres://user:password@host:5432/db) |
| `--bundled-postgres` | Run a single PostgreSQL pod in the cluster, for trying Everruns out (no backups or failover) |
| `--public-url <PUBLIC_URL>` | Public address people use, such as https://everruns.example.com; adds an Ingress. |
| `--ingress-class <INGRESS_CLASS>` | Ingress class for the public address, such as nginx. |
| `--chart <CHART>` | Chart to install: a local path such as charts/everruns, or an OCI reference. |
| `--chart-version <CHART_VERSION>` | Chart version for a new install (default: this CLI's version) |
| `--image-tag <IMAGE_TAG>` | Image tag to run, such as v0.51.0 or latest (default: the chart's release) |
| `--set <KEY=VALUE>` | Extra chart value (key=value), as with helm --set; kept on later runs. Repeatable. |
| `-f`, `--values <FILE>` | Extra values file, as with helm --values. Repeatable. |
| `--port <PORT>` | Local port for the port-forward when there is no public URL (default 9300) |
| `--timeout <TIMEOUT>` | How long to wait for the pods, such as 15m. |
| `-y`, `--yes` | Accept the default for every question, the cluster context included. |
| `--no-open` | Do not open the browser. |


## k8s status

Show the install: where it is, its pods, and which CLI profile points at it.

```bash
everruns k8s status [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |


## k8s logs

Show the pods' logs.

```bash
everruns k8s logs [OPTIONS] [COMPONENT]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |
| `<COMPONENT>` | Only this part (server, worker, ui, proxy, nats, postgresql, ...) |
| `--no-follow` | Print what is there and exit instead of following. |


## k8s open

Open the UI in the browser (through a port-forward when there is no public URL)

```bash
everruns k8s open [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |


## k8s port-forward

Keep a port-forward open so the CLI and browser reach an install without a public URL.

```bash
everruns k8s port-forward [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |


## k8s doctor

Check the tools and the cluster, and run the system doctor on the server.

```bash
everruns k8s doctor [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |


## k8s password

Print the admin password the chart generated.

```bash
everruns k8s password [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |


## k8s upgrade

Move the install to this CLI's chart version (or --chart-version), backing up a bundled database first.

```bash
everruns k8s upgrade [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |
| `--chart-version <CHART_VERSION>` | Chart version to move to, such as 0.52.0 (default: this CLI's version) |
| `--no-backup` | Skip the backup of a bundled database taken before the upgrade. |
| `--timeout <TIMEOUT>` | How long to wait for the pods, such as 15m. |
| `-y`, `--yes` | Do not ask, also when the move is to an older version. |


## k8s uninstall

Remove the workloads. The database, its volumes and the generated secrets are kept.

```bash
everruns k8s uninstall [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |


## k8s reset

Delete the install: workloads, generated secrets, volumes and the CLI profile. Asks first.

```bash
everruns k8s reset [OPTIONS]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Install name, also the Helm release name. |
| `-y`, `--yes` | Do not ask for the install name (for scripts) |

