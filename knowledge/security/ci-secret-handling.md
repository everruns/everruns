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

- **A false alarm is fixed at the source.** A documented API example lands verbatim in a public log
  whenever a contract test fails and dumps the catalog, and an `sk-` prefixed placeholder there is
  indistinguishable from a live key. Allowlisting repository content would make a secret committed
  there invisible to the log scan too, so the examples give up the credential shape instead, guarded
  by a fixture in [`scripts/test-actions-log-secret-scan.sh`](../../scripts/test-actions-log-secret-scan.sh).

- **When the source is immutable, cut the route instead.** A commit message is history: a
  credential shape quoted in one — a placeholder, a key being discussed — cannot be reshaped after
  the fact. `release.yml`'s `check-release` handed `github.event.head_commit.message` to a step
  through `env:`, and the runner prints an `env:` entry verbatim, so every merged PR description was
  republished into a public log on every push to `main`. That is what turned the sweep red on
  commit `d6f65f8`, whose body quotes the very placeholder the commit removed. The job only needed a
  boolean, so the job now checks the subject read from the checked-out commit and the message never
  reaches a log-visible environment entry. The subject remains quoted data in a case-sensitive Bash
  comparison, not shell code. The general rule still points the other way — binding event text to
  `env:` is what keeps it out of `run:` where it would be an injection vector — so this is not a
  blanket prohibition: prefer a derived value when the step does not need the text itself.

- **A prefix rule needs a left boundary.** `sk-` with no boundary matched inside
  the ordinary word "ask-", so a PR body linking
  `linear.app/.../EVE-1053/ask-user-capability-contract-schema-and-knowledge-spec`
  reported as an OpenAI key. Docker Build echoes PR bodies, so that fired on
  every branch of a nine-issue project. A real credential always begins at a
  token boundary, so requiring one costs no detection and removes a whole class
  of alarm whose cause is invisible from the finding — the scanner withholds the
  value, correctly, which leaves a High finding nobody can act on.

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

A finding that turns out to be a placeholder is still work: reshape the placeholder so it stops
matching, then delete the logs carrying it. Waiting it out is not a response — the sweep reads a
75-minute window, so the alarm clears itself once the run ages out, and recurs at full severity the
next time anything dumps that value. A High finding that resolves to nothing and then goes away on
its own is how a detector stops being read.
