# Bashkit workspace (Serve, experimental)

A small [Serve](../../../crates/serve) app that creates a report with Bashkit
and reads it again after restarting the server.

## Try it

You need Rust, Python 3.10+, and `OPENAI_API_KEY` set in your environment
with access to `gpt-6-luna`. From the repository root:

```sh
cargo build -p serve-example-bashkit-workspace
python3 examples/serve/bashkit-workspace/demo.py target/debug/bashkit-workspace
```

The script starts and stops Serve for you. It makes four real model turns
(normal provider charges apply), creates a report containing `apples` and
`pears`, and checks that it survives a restart. A successful run ends with:

```text
PASS: files survive turns and restart; sessions are isolated; shell variables reset.
```

It prints a temporary directory containing the resulting files and logs.
Your existing Serve data is untouched. If you use `CARGO_TARGET_DIR`, pass
that directory's binary path instead.

## How Bashkit is configured

Select the runtime in `serve.toml`:

```toml
[sandbox]
kind = "bashkit"
```

Serve installs `BashkitShell::new()` and supplies the session filesystem.
The Framework's default workspace policy is **read-only**. `src/agent.rs`
explicitly permits writes only under `/workspace/demo` through Serve's
`.customize(...)` hook:

```rust
builder.workspace_policy(
    everruns::WorkspacePolicy::builder()
        .allow_read("/")
        .allow_write("/workspace/demo")
        .build()
        .expect("valid demo workspace policy"),
)
```

There is no custom shell tool, Docker container, external database, or host
shell execution in the agent. Bashkit implements shell commands in process;
it does not provide every program installed on your machine. Shell HTTP is
disabled by default.

## Verification details

The script launches `start`, which requires a real model route. For other
provider configurations, see [Serve](../../../crates/serve/README.md#try-it).
Keep provider credentials in your environment, not in `serve.toml`, which
is embedded in the binary. Building the example or running its `manifest`
command does not call a model.

`demo.py` starts Serve on an unused port, sets a fresh API bearer key, and
uses only local HTTP for the client. The CLI listens on all interfaces;
the generated key protects its agent routes. The script stops its server
even on failure, and retains a fresh temporary directory for inspection.
It never reads or writes your existing `.serve/` data and ignores `SERVE_STORE`
so it cannot attach to a configured remote bucket.

The walkthrough checks actual `tool.completed` results and file contents,
not just the model's final response:

1. A Bashkit command creates `/workspace/demo/items.txt` containing three
   lines: `apples`, `pears`, `apples`.
2. A later turn executes
   `sort /workspace/demo/items.txt | uniq > /workspace/demo/report.txt`
   and reads the result. It also checks that an exported shell variable
   from the first call has reset.
3. The client stops Serve, verifies `report.txt` on disk, and restarts the
   same binary with the same `SERVE_DATA_DIR`. It sends a read-only `cat`
   command to the **original session ID** and checks the result again.
4. A new session executes a shell check proving neither file exists there.

The printed temporary directory contains
`server.log`, one JSON event trace per turn, and `data/` with Serve's stores
and session workspaces. The script prints the actual host path of
`report.txt`; `/workspace/demo/report.txt` is its **agent-visible** path,
not an absolute path in your checkout. Inspect the files, then delete only
that printed temporary directory when finished.

For a custom client, see the [Serve session API](../../../crates/serve/docs/wire-api.md).
`demo.py` shows the request bodies, event polling and error checks.

## Persistence boundaries

- Files persist across Bashkit calls and later turns in the same session.
  Shell variables, current directory and shell processes are not retained
  between tool calls. Use absolute paths or `cd` in each command.
- The default native `dev`/`start` host keeps separate directory workspaces
  under `SERVE_DATA_DIR/workspaces`, alongside the durable session stores.
  Retain the **whole data directory**, at the same absolute path. Starting
  with a fresh directory creates a fresh store; copying only the journal
  does not preserve workspace files.
- This demo verifies a graceful restart between completed turns with the
  same binary. It does not test crashes during writes, interrupted turns,
  upgrades, bucket replication or other hosting targets. `start` pins
  sessions to their build; a changed build can return `409`.
- Session filesystem isolation is separate from caller authorization.
  Serve's bearer authentication does not isolate sessions by caller: an
  authenticated operator can access other sessions through the API.
- An in-process `eval` without a data directory uses memory and does not
  demonstrate persistence across process restarts.

For embedding Bashkit directly in the Framework rather than serving an app,
see [bashkit-repo-agent](../../bashkit-repo-agent).

## Credential-free regression test

Check the client's disposable-store isolation without starting Serve:

```sh
python3 -m unittest discover -s examples/serve/bashkit-workspace/tests
```

The Serve test suite checks these storage boundaries with the real Bashkit
runtime and a scripted model confined to test code:

```sh
cargo test -p everruns-serve --lib bashkit_files_survive_turns_and_host_restart
```

It reopens the Serve host over the same data directory, checks shell output
across turns, verifies that shell variables reset, and checks a fresh session.
This is a host-reopen regression test; run `demo.py` to exercise real model
calls, HTTP transport and separate server processes together.
