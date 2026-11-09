# Attached CLI: drive a running session from another terminal

A host that serves its live session to a second process, and the attached CLI
that lists, watches, sends to and cancels it. Copy this crate to give your own
terminal host a `myagent attach` without writing a control plane: the socket,
the token and the permission checks come from `everruns::control` (feature
`control`, Unix only). Guide: [Attach to a Running Session](../../docs/framework/attach-control.md).

## Run it

Terminal 1, the host. It runs OpenAI `gpt-6.1-sol` and also takes prompts on
its own stdin:

```bash
OPENAI_API_KEY=... cargo run -p everruns-attached-cli -- host
```

Terminal 2 onward, the attached side (same OS user, no API key needed):

```bash
cargo run -p everruns-attached-cli -- list
cargo run -p everruns-attached-cli -- attach main --after 0   # replay, then follow live
cargo run -p everruns-attached-cli -- send main "write a haiku about sockets"
cargo run -p everruns-attached-cli -- send main --no-wait "now a long essay on ports"
cargo run -p everruns-attached-cli -- cancel main
```

`--dir <DIR>` selects the control directory on both sides; the default is
`$XDG_RUNTIME_DIR/everruns-attached-cli`, or a per-user directory under the
system temp dir.

What to expect: the attach terminal shows every user message (`> ...`), the
streamed reply, tool names, and `[turn cancelled]` after a cancel. A prompt
typed in the host terminal is cancellable from terminal 2 too, because the host
sends it through the registry. Ctrl-C in an attached terminal only detaches.
Stopping the host (Ctrl-C or EOF) ends attach streams with
`[detached: shutdown]` and removes the socket.

## How it is wired

- `src/lib.rs` `serve`: `ControlRegistry::register("main", &session)`,
  `ControlServer::start(dir, registry)`, and the host's own prompts through
  `registry.send` so their turns are tracked.
- `src/lib.rs` `run_attached`: one `ControlClient` call per subcommand, and
  `ControlEvent::render` for output.
- `src/main.rs`: argument parsing and the real provider.

## Who may attach

Only the user running the host: the directory is 0700, the socket 0600, and
both ends check kernel peer credentials. Each request also carries a token
from the 0600 `token` file, rotated every 15 minutes and revocable with
`ControlServer::revoke`. Details and the cancel semantics are in the guide.

## Tests

`tests/attach_flow.rs` runs the host and the attached commands in one process
with a simulated model: list, send and reply, replayed attach, a cancel of a
turn the host started, and clean shutdown. The binary always uses a real
model.
