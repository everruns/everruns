# everruns-worker

> Execution worker for the Everruns control plane.

`everruns-worker` is the worker binary that executes Everruns agent work. Workers
connect to the control-plane server over gRPC and run the agent loop,
reasoning, tool calls, and capability execution, while the server owns
persistence and coordination. It is an internal component of an Everruns
deployment, composed via the worker app builder.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. The control plane it connects to
lives in `everruns-server`.

## Usage

```bash
# Start PostgreSQL, server, worker, and UI together for local development
just start-all
```

## What It Provides

- Runs the Everruns agent loop against a control-plane server
- Connects to the server over the `everruns-internal-protocol` gRPC API
- Composable setup via the worker app builder

## Documentation

- [Architecture](https://docs.everruns.com/explanation/architecture/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
