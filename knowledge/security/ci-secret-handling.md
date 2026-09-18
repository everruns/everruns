---
type: Specification
title: "CI Secret Handling"
description: "Why a runtime-fetched secret must stay in one process, and the two layers that enforce and detect it."
tags:
  - everruns
  - security
  - ci
  - secrets
---
# CI Secret Handling

## Abstract

GitHub Actions masks a value in logs only when it flowed through `secrets.*`. Everruns fetches most
live-test credentials from Doppler at runtime instead, which is a second secret plane the runner
knows nothing about. This specification states the resulting rule, and the two layers that keep it
true.

This is the CI counterpart to [`secret-leak-guardrails.md`](secret-leak-guardrails.md), which covers
the same problem inside the product. Both land on the same shape — a deterministic layer that fails
closed and a broader layer that catches what the first cannot name.

## The rule

**A credential fetched at runtime stays in the environment of the one process that needs it.**

`doppler run -- <cmd>` satisfies this. Writing the value to `$GITHUB_ENV` or `$GITHUB_OUTPUT` does
not: those cross a step boundary, and the runner prints the resulting environment in the `env:`
group it emits for every later step. Nothing has to echo the value for it to reach the log.

`::add-mask::` is accepted as a fallback but is strictly weaker. Masking is substring-exact, so a
consumer that base64s, URL-encodes, or otherwise re-shapes the value defeats it.

## Why it is two layers

Prevention alone was not enough, because the two failure modes are different in kind:

| Layer | Catches | Cannot catch |
|---|---|---|
| [`scripts/test-workflow-secret-handling.sh`](../../scripts/test-workflow-secret-handling.sh) | the pattern being written, before it runs | a credential the workflow never names — a vendor error body, a value a failing test echoes |
| [`scripts/scan_actions_log_secrets.py`](../../scripts/scan_actions_log_secrets.py) | credentials that reached a log, whatever their route | anything in the window before the hourly sweep |

The static guard is the fail-closed half; the scanner is the backstop. Neither replaces reducing the
blast radius (see Residual risks).

## Detection design

Two decisions in the scanner are load-bearing and non-obvious:

- **The runner is the oracle.** It renders values it knows as `***`. A credential-named entry
  holding an opaque value *beside* those asterisks is unmasked by construction, so the primary rule
  needs no enrolled values and no vault access. Prefix rules cover formats that never pass through
  an env block.
- **The allowlist is derived, not maintained.** A value committed in a workflow file is public
  already and cannot be a leak. It is keyed on the value rather than the variable name, so the same
  variable carrying anything else still reports — a name-keyed allowlist would go blind at the next
  rotation. The blind spot this leaves, a real secret committed into a workflow file, belongs to
  GitHub secret scanning on repository content.

Entropy is deliberately not the discriminator. `debug-ubuntu-latest` scores 3.35, above any
threshold a hex key could clear, since hex caps at 4.0. The signal is an unbroken alphanumeric run,
which a credential has and a hyphenated setting name does not.

## Residual risks

- **Detection latency.** The sweep is hourly. It is scheduled rather than `workflow_run`-triggered
  because that trigger needs a literal list of workflow names, which goes stale silently when one is
  renamed or added.
- **`DOPPLER_TOKEN` is vault-wide.** Every job that installs the Doppler CLI can read every secret,
  so any leak is total rather than scoped. Per-config service tokens would cap this, and would
  reduce the severity of the next incident more than either layer above. Not yet done.
- **Public logs.** The repository is public, so a leaked value is readable by anyone for as long as
  the log exists. Detection shortens the time until *we* know, not until others could.

## Response

On a finding: rotate the named secret first, then delete the affected run logs
(`DELETE /repos/{owner}/{repo}/actions/runs/{run_id}/logs`). Deleting logs without rotating is not a
remedy — the value has already been readable.
