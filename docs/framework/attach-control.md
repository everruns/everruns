---
title: Attach to a Running Session
description: Let a second local process list a host's sessions, watch their output, send input and cancel turns, with same-user access, a rotating token and revocation.
---

A terminal host often wants `myagent attach` in a second terminal: see which
sessions are live, watch one's output, type into it, stop a turn that went the
wrong way. The Engine's sessions live in the host process, so the host serves a
small local control plane and the second process talks to it. The
`everruns::control` module (feature `control`, Unix only) provides both halves.

Stability: experimental.

```toml
everruns = { version = "*", features = ["control", "openai"] }
```

## Serve sessions from the host

Put the sessions a second process may reach in a `ControlRegistry`, then start
a `ControlServer` on a private directory. Nothing outside the registry is
reachable.

```rust
use everruns::control::{ControlRegistry, ControlServer, default_dir};
use everruns::{Agent, Engine, OpenAI};

# async fn host() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Keep answers short.")
    .provider(OpenAI::from_env()?)
    .model("gpt-6.1-sol")
    .build()?;
let session = Engine::new().create(agent);

let registry = ControlRegistry::new();
registry.register("main", &session);
let server = ControlServer::start(default_dir("myagent"), registry.clone())?;
println!("attach with: myagent attach --dir {}", server.attach_info().dir.display());

// The host's own prompts go through the registry, so an attached
// `cancel` can reach the turn they start.
let reply = registry.send("main", "Hello").await?.wait().await?;
println!("{}", reply.response);
# Ok(())
# }
```

`default_dir(app)` is `$XDG_RUNTIME_DIR/<app>` when set, otherwise
`<temp dir>/<app>-<uid>`. Keep the server value alive for as long as the host
serves; dropping it stops serving and removes the socket and token file.

A turn is cancellable from outside only if the registry knows its
`TurnHandle`. Send through `registry.send`, or report a send made elsewhere
with `registry.track(&session.id(), &sent)`.

## Drive it from a second process

`ControlClient` is the attached side. Each call opens one connection.

```rust
use everruns::control::{ControlClient, default_dir};

# async fn attached() -> Result<(), Box<dyn std::error::Error>> {
let client = ControlClient::new(default_dir("myagent"));

for session in client.list().await? {
    println!("{} {} {:?}", session.label, session.id, session.active_turn);
}

// Start a turn (or steer the running one) and wait for its reply.
let (_, turn) = client.send_and_wait("main", "Summarize the plan").await?;
println!("{}", turn.response);

// Stop whatever is running.
client.cancel("main").await?;

// Watch output; `Some(0)` replays the conversation first, `None` starts live.
let mut stream = client.attach("main", Some(0)).await?;
while let Some(event) = stream.next().await? {
    if let Some(text) = event.render() {
        print!("{text}");
    }
}
# Ok(())
# }
```

Sessions are addressed by id or by a unique label. `attach` streams user
messages from every sender, assistant text, tool names and turn boundaries.
Tool arguments and results are not forwarded. Each event carries the durable
`sequence` when it has one, so a client that disconnected or saw a `lagged`
event re-attaches with `Some(last_sequence)` and misses nothing durable.

The [`attached-cli` example](https://github.com/everruns/everruns/tree/main/examples/attached-cli)
is a complete host and attached CLI in about 200 lines. Copy it to start.

## Who may attach

Access is local and has two independent gates:

- **Same OS user.** The control directory is created with mode 0700 and the
  socket with 0600. The server reads each connection's kernel peer credentials
  and refuses a different uid before reading a byte. The client checks the
  directory and token file belong to it and are private, and that the process
  behind the socket runs as the same user, before it sends the token or any
  input. A directory that already exists with group or other permissions, or
  another owner, is refused by both ends.
- **Bearer token.** 32 random bytes, hex encoded, in the 0600 `token` file
  next to the socket. Every connection's first line carries it, and the server
  compares it in constant time.

There is no way to grant another user access. A host that needs remote or
multi-user access should serve sessions over an authenticated protocol such as
[AG-UI](/framework/ag-ui/) or [serve](/framework/serve/) instead.

## Token lifecycle

| Event | Effect |
| --- | --- |
| `ControlServer::start` | Issues a token and writes the token file. |
| Every `token_ttl` (default 15 minutes) | Issues a new token. The old one stops opening connections at once; the client re-reads the file once when a token is refused, so rotation is invisible to it. Open attach streams continue. |
| `rotate_token()` | Same as a scheduled rotation, on demand. Also re-opens a revoked server. |
| `revoke()` | Deletes the token and the file. Every open attach stream and waiting send ends with `revoked`, and new connections are refused until `rotate_token()`. |
| Dropping the server | Stops serving, ends open streams with `shutdown`, and removes the socket and token file. |

```rust
use std::time::Duration;
use everruns::control::{ControlRegistry, ControlServer};

# fn configure(registry: ControlRegistry) -> std::io::Result<()> {
let server = ControlServer::builder("/run/user/1000/myagent")
    .token_ttl(Some(Duration::from_secs(60)))
    .max_connections(8)
    .start(registry)?;
// Someone copied the token somewhere it should not be:
server.revoke();
server.rotate_token()?;
# Ok(())
# }
```

## Cancel semantics

`cancel` targets the session's active tracked turn, the most recent one sent
through the registry that has not finished, and returns its identity (the turn
id, or the accepting message id while the turn is still being assigned). It
behaves exactly like `TurnHandle::cancel`:

- The turn stops at its next await point; running tool work is dropped and the
  session records `turn.cancelled`.
- Messages that steered into that turn end with it.
- The runtime records a short user message noting the cancellation, so the
  next turn knows the work was stopped. Attached streams show it like any
  other user message.
- The session stays usable; the next `send` starts a new turn.
- A session with no running tracked turn answers `no_active_turn`.

Disconnecting never cancels anything. Dropping an attach stream, a client that
exits while waiting on `send_and_wait`, a revocation or a server shutdown all
leave the session's turns running.

## Protocol

One newline-delimited JSON request per connection, answered by one or more
JSON lines: `{"token": "...", "command": {"op": "list"}}`, with `send`,
`cancel` and `attach` taking a `session` field. Errors are
`{"type": "error", "code": "...", "message": "..."}`, with codes
`unauthorized`, `bad_request`, `not_found`, `ambiguous`, `no_active_turn`,
`session_closed`, `busy` and `failed`. Requests are capped at 256 KiB and must
arrive within 10 seconds; connections beyond `max_connections` get `busy`. The
serde types in `everruns::control` are the wire format, so a client in another
language can speak it.
