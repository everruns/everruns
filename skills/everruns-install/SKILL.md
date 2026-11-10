---
name: everruns-install
description: Install, upgrade, check, or remove a self-hosted Everruns on the person's own machine, server, or Kubernetes cluster with the everruns CLI. Use when the person asks to install, run, upgrade, or troubleshoot their own Everruns. Not for the demo instance or for Rust framework use (everruns-framework).
license: MIT
compatibility: A shell on the person's machine. Docker for a laptop or server; kubectl and helm for Kubernetes.
metadata:
  category: platform
  version: "1.0"
---

# Install Everruns

Everruns is free, MIT-licensed, and runs on infrastructure the person controls.
One CLI does every install: `everruns docker up` on a machine with Docker,
`everruns k8s up` on a Kubernetes cluster. Both ask the same questions, take
every answer as a flag, check prerequisites first, and sign the CLI in at the end.

Use this skill only for an install the person asked for. Installing changes
their machine or cluster, so the rules below are not optional.

## Rules

1. **Show the plan, then wait for a yes** before any command that changes a
   cluster, a server, or Docker state: where it goes (kube context and namespace,
   SSH host, or "this machine"), which database, which public URL, which
   provider. Read-only checks (`doctor`, `status`, `kubectl config
   current-context`) need no approval.
2. **Keep secrets out of the conversation.** Never ask the person to paste an
   API key, database URL with a password, or token into chat. Ask them to export
   it in their own shell (`OPENAI_API_KEY`, `DATABASE_URL`) and pass it by name:
   `--database-url "$DATABASE_URL"`. Do not print generated passwords; tell the
   person which command shows them (`everruns k8s password`, or the `.env` path
   `docker up` reports).
3. **Use `--yes` only after the person approved the plan**, and always together
   with explicit flags for every value they chose, so `--yes` never picks a
   cluster or database for them.
4. **Report what you verified.** Finish with the checks under "Verify" and say
   which passed. Do not say the install worked before they pass.

## 0. Install the CLI

Check `everruns --version`. If missing:

- macOS or Linux with Homebrew: `brew tap everruns/tap && brew install everruns`
- Otherwise: `cargo install --git https://github.com/everruns/everruns everruns-cli`

## 1. Pick the target

| The person says | Target | Section |
| --- | --- | --- |
| this machine, my laptop, locally | Docker on this machine | 2 |
| my server, a VM, over SSH | Docker Compose on that host | 3 |
| my cluster, Kubernetes, k8s, EKS/GKE/AKS | Kubernetes | 4 |

If unclear, ask once. Do not choose the demo instance; it is a demo, not an
install target.

## 2. This machine (Docker)

Run `everruns docker doctor`. Fix what it reports (Docker missing, stopped, too
old, no permission), or tell the person what to do.

Plan to confirm: install name (default `default`), who signs in (`single`:
just them, with a password), admin email, provider.

```bash
everruns docker up --yes --admin-email <email> --provider <openai|anthropic|gemini|openrouter|skip>
```

The provider key is read from the person's environment. If none is set, use
`--provider skip`; they can add one later under **Models → Providers**.

## 3. A server (Docker Compose over SSH)

Install the CLI on the server and run section 2 there through SSH. For a
public address add `--public-url https://<host>` (needs DNS pointing at the
server) or `--tunnel tailscale|cloudflare|ngrok` when the server has no public
IP. `--auth none` is refused with a public URL; keep `single` or `accounts`.

## 4. Kubernetes

Read-only checks first:

```bash
kubectl config current-context
everruns k8s doctor --context <context>
```

Plan to confirm, in one message. Fill in what the person did not say with
defaults and say so, rather than asking question by question:

- cluster: kubectl's current context, unless they named another
- namespace: `everruns`
- database: a single PostgreSQL pod in the cluster (`--bundled-postgres`, for
  trying it out: no backups or failover), unless they have their own
  PostgreSQL 15+, which they put in their shell as `DATABASE_URL`
- public URL and ingress class, or none (then `everruns k8s port-forward`)
- who signs in and the admin email
- provider, from the key in their environment

After the yes:

```bash
everruns k8s up --yes --context <context> --namespace <namespace> \
  (--bundled-postgres | --database-url "$DATABASE_URL") \
  --admin-email <email> --provider <provider> \
  [--public-url https://<host> --ingress-class <class>]
```

It waits for the pods and signs the CLI in as profile `k8s`. If pods do not
become ready, run `everruns k8s status` and `everruns k8s logs <component>
--no-follow`, explain the cause, and propose a fix; do not retry blindly.

## Upgrade and remove

- Upgrade: `everruns docker upgrade` or `everruns k8s upgrade` (backs up a
  bundled database first). Show the version change and ask first.
- Remove workloads, keep data: `everruns k8s uninstall`. Deleting data
  (`everruns k8s destroy`, `everruns docker destroy`) is permanent: only on an
  explicit request, and the person types the install name themselves.

## Verify

1. `curl -fsS <url>/health` returns OK (`http://localhost:9300` without a public URL).
2. `everruns whoami` answers as the admin email.
3. The person opens the URL and signs in. Tell them which command prints the password.

Then suggest one next step without doing it: connect their coding agent
(`everruns-mcp`), or build a first agent (`everruns-build-agent`).

## Reference

- On your machine: <https://everruns.com/docs/getting-started/run-locally/>
- Kubernetes: <https://everruns.com/docs/getting-started/kubernetes/>
- CLI reference: <https://everruns.com/docs/reference/cli/k8s/>
