# everruns-cli

> Command-line interface for Everruns.

`everruns-cli` is the `everruns` command-line tool for driving an Everruns
deployment from a terminal or a script: managing agents and sessions, running
turns, and streaming events. It supports text and JSON output for scripting and
resolves credentials from a config file with environment-variable overrides.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents.

## Usage

```bash
# Build and run from the workspace
cargo run -p everruns-cli -- --help

# Point at a deployment and list sessions as JSON
everruns --output json sessions list
```

Credentials are read from the platform config directory
(`<config>/everruns/credentials.json`) and can be overridden with environment
variables.

## What It Provides

- Agent and session management from the command line
- Running turns and streaming events
- Text and JSON output modes for interactive use and scripting

## Documentation

- [CLI feature overview](https://docs.everruns.com/features/cli/)
- [Automate with the CLI](https://docs.everruns.com/how-to/automate-with-the-cli/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
