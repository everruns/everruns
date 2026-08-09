---
title: Session History and Resume
description: Read bounded conversation history and continue Framework sessions without importing host or runtime storage APIs.
---

Every Framework session has a typed `SessionId`. Keep that value when an
application may need to reopen the conversation:

```rust
use everruns::{Agent, Model, SessionId};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Remember the conversation.")
    .model(Model::simulated("Acknowledged."))
    .build()?;

let mut session = agent.session();
let session_id: SessionId = session.session_id();
session.run("My project is Atlas.").await?;

drop(session);
let mut resumed = agent.resume(session_id).await?;
resumed.run("Continue with that project.").await?;
# Ok(())
# }
```

`resume` verifies the ID against the Agent's configured session catalog. It
does not infer identity from a non-empty transcript: a valid session can have no
messages, and stray events do not create a resumable session. An unknown ID
returns a typed not-found error. The resumed session uses the current Agent
configuration—model, instructions, tools, and workspace—not a serialized copy
of the old configuration.

## Read bounded history

`Session::history` creates an owned query. Calling `page` returns at most 100
messages by default in canonical event-sequence order, oldest first:

```rust
# use everruns::{Agent, Model};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder()
#     .instructions("Be concise.")
#     .model(Model::simulated("Done."))
#     .build()?;
# let session = agent.session();
let page = session.history().page().await?;
for message in &page.messages {
    println!("{:?}: {}", message.role, message.text());
}
# Ok(())
# }
```

Set a smaller or larger page size with `limit`. The maximum is 256 messages;
an excessive value returns `HistoryError::InvalidLimit` with the allowed
maximum. A page never claims to contain the entire transcript. Continue from
its opaque cursor:

```rust
# use everruns::{Agent, Model};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder()
#     .instructions("Be concise.")
#     .model(Model::simulated("Done."))
#     .build()?;
# let session = agent.session();
let first = session.history().limit(25)?.page().await?;
if let Some(cursor) = first.next_cursor {
    let second = session.history().limit(25)?.after(cursor)?.page().await?;
    // `second` continues the same stable snapshot.
}
# Ok(())
# }
```

`HistoryCursor` is opaque, session-bound, and safe to store as a string with
`Display` and restore with `FromStr`. A cursor fixes the snapshot's high-water
mark: events appended after the first page do not appear midway through that
page walk. Start a new query to see them. Passing a malformed, cross-session,
expired, or incompatible cursor returns a distinct typed history error.
History projection also applies a bounded raw-event replay safety limit; an
unusually lifecycle-heavy snapshot that exceeds it returns
`HistoryError::HistoryTooLarge` instead of performing an unbounded scan.

For callers that intentionally walk the whole snapshot, `pages` is a lazy
convenience that still reads one bounded page at a time:

```rust
# use everruns::{Agent, Model};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder()
#     .instructions("Be concise.")
#     .model(Model::simulated("Done."))
#     .build()?;
# let session = agent.session();
let mut pages = session.history().limit(50)?.pages();
while let Some(page) = pages.next_page().await? {
    for message in page.messages {
        println!("{}", message.text());
    }
}
# Ok(())
# }
```

After the final page, `next_page` remains fused and returns `None`. It does not
re-read the backend or produce repeated empty terminal pages.

## Choose a persistence lifecycle

The default Agent owns an in-memory event log. It needs no database, network,
credentials, or filesystem access. Sessions can be dropped and resumed through
that Agent or one of its clones, but creating a new Agent starts a new volatile
history store. Process exit loses it.

Enable `local` and configure a trusted application data directory when sessions
must survive a new Agent or process:

```rust
use everruns::{Agent, LocalConfig, Model};

let agent = Agent::builder()
    .instructions("Remember the conversation.")
    .model(Model::simulated("Ready."))
    .local(LocalConfig::new(".everruns-data"))
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

The local profile stores a durable session catalog and crash-durable canonical
event log alongside its workspace, task, and schedule state. Build another
Agent with the same trusted data directory and call `resume(session_id)`. A new
session is made durable by its first async operation (`run`, `inspect`, or a
history page read); merely allocating a synchronous handle does not commit it.
The local profile is for one embedded process at a time. Do not write or edit
its files as application data: messages are a read-only projection of committed
events, and the storage formats are not Framework APIs.

History does not contain model credentials or application secrets unless an
application deliberately includes them in message content or event metadata.
Choose and protect the local data directory accordingly.
