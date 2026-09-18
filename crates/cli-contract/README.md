# everruns-cli-contract

> One `everruns` command line, shared by the CLI and the agent's shell.

`everruns-cli-contract` holds the `everruns` command grammar as data, and the
single function that turns it into a `clap::Command`. A caller learns one CLI:
what a person types in a terminal and what an agent types in its shell are the
same words, with the same flags, the same short options, the same positionals
and the same help.

Part of the [Everruns](https://everruns.com) ecosystem. `everruns-cli` mounts
these commands into its own tree, and the server builds the agent-facing
command tree from the same values.

## Quick Example

```rust
use everruns_cli_contract::{ArgKind, ContractArg, ContractCommand, ContractExample};

let command = ContractCommand {
    wire_name: "list_agents".into(),
    path: vec!["agents".into()],
    verb: "list".into(),
    description: "List agents in the organization.".into(),
    method: "GET".into(),
    http_path: "/v1/agents".into(),
    args: vec![ContractArg {
        field: "limit".into(),
        long: "limit".into(),
        short: None,
        position: None,
        kind: ArgKind::Integer,
        required: false,
        help: Some("Maximum rows to return.".into()),
        choices: vec![],
    }],
    examples: vec![ContractExample {
        intent: "List the ten most recent agents".into(),
        command: "everruns agents list --limit 10".into(),
    }],
};

assert_eq!(command.spelling(), "agents list");
assert_eq!(
    command.clap_command("everruns agents list").get_name(),
    "everruns agents list"
);
```

## What It Provides

- The `everruns` grammar as owned data: nouns, verbs, arguments, examples
- One `clap::Command` builder, so neither surface keeps its own copy
- Worked examples as an intent plus a command line, rendered under `--help`
- A diffable rendering of a clap tree, used to pin both surfaces against drift

## Why the grammar is data

`everruns-cli` spells its commands with clap derive, which is the right tool
when a human writes each one. It cannot be the shared definition, for two
reasons that only appear once both consumers are real:

- The control plane owns dozens of routed commands and the CLI hand-writes a
  fraction of them. Deriving the rest would mean hand-writing structs whose
  fields already exist, as the parameter types the commands deserialize.
- The agent-facing tree is assembled in a worker from commands fetched at
  runtime. A `&'static` derive tree cannot be built from fetched data;
  `clap::Command` is a runtime builder over owned strings, and can.

So the presentation a human chose — the short option, the bare word, the order
— is declared next to each command, everything else comes from the parameter
schema the command already publishes, and both consumers build their
`clap::Command` from the result.

## Conventions the types enforce

- **A long flag is kebab-case**, and also answers to the parameter's own
  snake_case name, so a script written against either keeps working.
- **An example is an intent and a command line**, not a bare command line. An
  agent reading `--help` is choosing between commands, not recalling one it
  already knows.
- **Colour is off.** The workspace links clap with its default features, and
  cargo unifies that across a build, so a command that does not say
  `ColorChoice::Never` emits escape bytes into terminals and tool results alike.

## Documentation

- [Command tree](https://docs.everruns.com/)
- [API reference](https://docs.rs/everruns-cli-contract)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
