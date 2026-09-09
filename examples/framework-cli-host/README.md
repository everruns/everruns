# Framework host with a command tree

A Framework application whose own operations appear as `everruns <noun> <verb>`
inside the agent's shell.

Nothing here is server-specific. This application has no control plane, no
database, and no domain-command catalog; it owns three operations over an
in-memory fleet. It implements `CliCommandSource`, hands that to the runtime,
and the agent gets the same grammar, the same bounded help, and the same error
behaviour that the hosted product's much larger tree provides.

## Run it

The example always calls a real model. There is no simulator mode: a simulator
can be told to emit the exact commands we hoped for, which makes the output a
recording of our own script rather than evidence that an agent found the CLI.

```bash
# Either key works; Anthropic wins if both are set.
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-cli-host
OPENAI_API_KEY=...    cargo run -p everruns-framework-cli-host

# Pick one explicitly when both keys are exported but only one account can
# currently serve a request.
cargo run -p everruns-framework-cli-host -- --provider openai

# Any OpenAI-compatible endpoint: OpenRouter, Fireworks, vLLM, a gateway.
OPENAI_API_KEY=... OPENAI_BASE_URL=https://openrouter.ai/api/v1 \
  OPENAI_MODEL=openai/gpt-4o-mini cargo run -p everruns-framework-cli-host

# Any trailing words become the prompt.
cargo run -p everruns-framework-cli-host -- \
  "Scale the api service to 4 replicas, then list the whole fleet."
```

## What a run shows

Three things, on purpose: the commands the agent typed, the bytes the
`everruns` builtin actually returned, and the application's own state
afterwards. The last is the load-bearing part, since a model can claim any
scale in prose but only the builtin can move `api` from 2 replicas to 4.

A real run, `gpt-4o-mini` via OpenRouter, prompt `"Scale the api service to 4
replicas."`. The model guesses a flag, gets a real error, reads the leaf's
help, and recovers:

```console
$ everruns fleet --help
everruns fleet
  Services this deployment runs, and their scale.

Usage: everruns fleet <command> [--flags]

Commands:
  get    Show one service.
  list   List services and their replica counts.
  scale  Set a service's replica count.

$ everruns fleet scale --service api --replicas 4
missing --name
[exit 1]

$ everruns fleet scale --help
everruns fleet scale
  Set a service's replica count.

Usage: everruns fleet scale --name <service> --replicas <count>

Examples:
  everruns fleet scale --name api --replicas 4

$ everruns fleet scale --name api --replicas 4
{"name":"api","replicas":4,"scaled":true}

== Fleet state after the turn ==
  api: 4
```

That recovery loop is the whole point of the surface, and it is why the
transcript prints stderr and a non-zero exit rather than stdout alone: the
failed command is the interesting half.

A live model picks its own commands, so runs differ. Whether one finds and
drives the CLI *reliably* is a population question that no single run answers;
the `cli-tree` cases in `evals/platform-capability` measure that.

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

`tests/cli_in_session.rs` drives the real bash tool directly, with no model and
no simulator: a simulator told to emit the commands a test hoped for makes the
run a recording of that test's own script. The load-bearing assertion is that a
`scale` command moves the application's own state from 2 replicas to 4. The
rest cover what a live model actually needs — a leaf rendering its own flags, a
wrong argument form failing with a message that names the fix, and a host that
supplies no source getting no builtin at all.
