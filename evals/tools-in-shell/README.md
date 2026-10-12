# Tools in Shell eval

A [Mira](https://github.com/everruns/mira) study of whether a model finds and
calls a tool it cannot see. With the `tools_in_shell` capability the agent's
tools leave its tool list and become one shell command, `tools`; the only ways
to find one are `tools --help`, a source's help, and `tools search`. The
capability pays off only if models find tools about as fast as they did with
direct tool schemas, and this study is how that gets measured rather than
assumed.

## What runs

Each case builds an in-process Framework agent with `bashkit_shell` and
`tools_in_shell` over a small fake registry ([`src/registry.rs`](src/registry.rs)):
three MCP-style sources (`crm`, `tickets`, `calendar`) and two top-level tools,
plus the Framework's own built-in tools, which go behind `tools` too.
Everything between the model and the fake tools is the shipped code: the
capability's prompt addition, the hook that hides the registry and names the
sources on `bash`, the builtin's help, search, input parsing and error
envelopes, and the per-call policy the turn engine installs. Only the tools
are fake, so a run needs one model key and no server.

A call from a script is not its own tool event, so each fake tool records the
call it received; scorers grade that log, not the model's claims.

## Cases

| Case | What it measures |
|---|---|
| `tis-find-by-keyword` | a top-level tool found from a plain-language need that names neither tool nor source |
| `tis-find-by-source` | a tool found through its source's listing, with input in the documented format |
| `tis-choose-between-similar` | `search-articles` chosen over `search-tickets` on the same source |
| `tis-json-input` | one JSON object with an enum and an array, passed right the first time |
| `tis-recover-wrong-name` | the person names `crm get-contact`, which does not exist; the `unknown_command` error should lead to `find-contact` in one retry |
| `tis-chain-two-tools` | one tool's output (a contact id) fed into another |

## Counting calls

Calls are counted the way the platform-capability study counts `everruns`
calls, with the same classifier, included by path from
[`../platform-capability/src/friction.rs`](../platform-capability/src/friction.rs):

| Class | Meaning here |
|---|---|
| `help` | every `tools` invocation only read: `tools --help`, `tools <source>`, `tools <tool> --help`, or `tools search …` |
| `rejected` | the builtin refused the call: `unknown_command` (no such tool or source) or `invalid_input` (the tool's schema refused the object) |
| `real` | a tool ran and nothing was refused; a tool's own `tool_error` is not a refusal of the surface |
| `other` | the call never invoked `tools` |

They land as transcript metrics (`friction.*`), as the informational
`tools_friction` score, as a per-case and overall table after `--run`, and,
with `--friction-report <path>` (or `EVERRUNS_EVAL_FRICTION_REPORT`), as one
JSON line per run with the commands tried, each refusal, and the help pages
read. The same tuning loop applies (the `tune-cli-help` agent skill), with the
`tools` help and tool descriptions in place of the command tree's.

## Run

```bash
export EVERRUNS_EVAL_TARGETS=openrouter/openai/gpt-5.5,openrouter/z-ai/glm-5.2
export EVERRUNS_EVAL_TRIALS=3
doppler run --command './target/debug/tools_in_shell --run --friction-report friction.jsonl'
mira --bin tools_in_shell run --preset smoke
```

Targets are `anthropic/<model>`, `openai/<model>` or
`openrouter/<vendor>/<model>`; unset, one OpenRouter model runs. A missing key
is an infra error (N/A), not a failure.

## What the first runs should watch

- **The root help is long enough to be cut.** The bash tool's default `auto`
  output keeps the head and tail of a long success. With the Framework's
  built-in tools listed as top-level `tools` commands, `tools --help` here
  exceeds that window and the `Servers:` list in its middle is dropped from what
  the model reads (the offline test shows it). The `bash` description still
  names the sources, so this may cost nothing; if `tis-find-by-source` or the
  `crm` cases show extra help reads, this is the first suspect.
- **Rejections on `tis-recover-wrong-name` are expected, once.** More than one
  means the error text is not pointing at `tools crm --help` or `tools search`
  clearly enough.

## Offline checks

`cargo test` runs no model and needs no key. It checks the dataset names real
registry tools, the scorers' matching rules, and the classifier against the
builtin's real envelopes, and drives one scripted run through the whole stack
with a simulated model: help, search, a wrong name, a refused input, then the
right call, with the friction count and the call log both checked.
`.github/workflows/tools-in-shell-evals.yml` runs them in CI.
