# Framework host with a command tree

A Framework application whose own operations appear as `everruns <noun> <verb>`
inside the agent's shell.

Nothing here is server-specific. This application has no control plane, no
database, and no domain-command catalog; it owns three operations over an
in-memory fleet. It implements `CliCommandSource`, hands that to the runtime,
and the agent gets the same grammar, the same bounded help, and the same error
behaviour that the hosted product's much larger tree provides.

```console
$ cargo run -p everruns-framework-cli-host -- --offline
Prompt: List the fleet, then scale the api service to 4 replicas.

== Agent's shell session ==
  $ everruns fleet --help
  everruns fleet
    Services this deployment runs, and their scale.

  Usage: everruns fleet <command> [--flags]

  Commands:
    get    Show one service.
    list   List services and their replica counts.
    scale  Set a service's replica count.

  Run `everruns fleet <command> --help` for flags.

  $ everruns fleet list
  {"services":[{"name":"api","replicas":2},{"name":"scheduler","replicas":1},{"name":"worker","replicas":1}]}

  $ everruns fleet scale --name api --replicas 4
  {"name":"api","replicas":4,"scaled":true}

== Response ==
Listed the fleet and scaled api from 2 replicas to 4.

== Fleet state after the turn ==
  api: 4
  worker: 1
  scheduler: 1

turn success: true | iterations: 4 | tool calls: 3
```

The run prints three things on purpose: the commands the agent typed, the bytes
the `everruns` builtin actually returned, and the application's own state
afterwards. The last one is the load-bearing part — a model can claim any scale
in prose, but only the builtin can move `api` from 2 replicas to 4.

## Run it

```bash
# Deterministic, no key, no network: the simulator issues the shell calls.
cargo run -p everruns-framework-cli-host -- --offline

# Against a real model. Either key works; Anthropic wins if both are set.
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-cli-host
OPENAI_API_KEY=...    cargo run -p everruns-framework-cli-host

# Any trailing words become the prompt, so you can drive a mutation and check
# it against the fleet state printed afterwards.
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-cli-host -- \
  "Scale the api service to 4 replicas, then list the whole fleet."
```

In `--offline` mode the simulator issues those shell calls deterministically,
so what is proven is the surface, not the model's judgement: the tree resolves,
help renders, and the mutation reaches the application. Whether a model *finds*
the CLI unprompted is a separate question, measured by the `cli-tree` cases in
`evals/platform-capability`.

## How it is wired

Three pieces, in `src/lib.rs` and `src/main.rs`:

1. `FleetCommands` implements `CliCommandSource` — the commands, their tree
   positions, one-line node descriptions, and their usage text.
2. The runtime is given that source through
   `with_tool_context_extensions_factory`, as a `CliCommandSourceHandle`.
3. The stock `BashkitShellCapability` is registered as usual.

The builtin appears only because step 2 happened. A host that supplies no
source gets no `everruns` command at all, rather than a shell advertising a
tree it cannot serve — `tests/cli_in_session.rs` covers that case explicitly.

## What the tests prove

`tests/cli_in_session.rs` runs real turns through the real interpreter. The
load-bearing assertion is that a `scale` command moves the application's own
state from 2 replicas to 4: a simulated final message can claim anything, but
only the builtin can move that number.
