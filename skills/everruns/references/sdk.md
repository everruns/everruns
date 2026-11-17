# Call Everruns from code

The SDKs are typed clients over the Everruns API, with the same shape in all
three languages: sub-clients for `agents`, `sessions`, `messages`, `events`,
workspace files, budgets and more, async throughout, and SSE streaming that reconnects
on its own.

| Language | Install |
|---|---|
| Python 3.10+ | `pip install everruns-sdk` |
| TypeScript, Node 18+ | `npm install @everruns/sdk` |
| Rust | `cargo add everruns-sdk` |

## Auth

The clients read `EVERRUNS_API_KEY` (a personal access token, `evr_pat_…`,
created under **Settings > Personal Access Tokens**), `EVERRUNS_API_URL`
(the deployment origin plus `/api`, such as `http://localhost:9300/api` for
`everruns docker up`) and optionally
`EVERRUNS_ORG_ID`. Keep the token in the environment or a secret store, never
in code; do not ask the user to paste it into the chat.

## Run an agent and read the reply

Python:

```python
import asyncio
from everruns_sdk import Everruns

async def main():
    client = Everruns()  # reads EVERRUNS_API_KEY, EVERRUNS_API_URL
    session = await client.sessions.create(agent_name="researcher")
    await client.messages.create(session.id, "Summarize today's top HN stories.")
    async for event in client.events.stream(session.id):
        if event.type == "output.message.completed":
            print(event.data)
        if event.type in ("turn.completed", "turn.failed"):
            break
    await client.close()

asyncio.run(main())
```

TypeScript:

```typescript
import { Everruns } from "@everruns/sdk";

const client = Everruns.fromEnv();
const session = await client.sessions.create({ agentName: "researcher" });
await client.messages.create(session.id, "Summarize today's top HN stories.");
for await (const event of client.events.stream(session.id)) {
  if (event.type === "output.message.completed") console.log(event.data);
  if (event.type === "turn.completed" || event.type === "turn.failed") break;
}
```

Rust:

```rust
use everruns_sdk::{CreateSessionRequest, Everruns};

let client = Everruns::from_env()?;
let session = client
    .sessions()
    .create_with_options(CreateSessionRequest::new().agent_name("researcher"))
    .await?;
client.messages().create(&session.id, "Summarize today's top HN stories.").await?;
```

A session created from an agent runs on that agent's harness. Reuse the session
for follow-up messages so the agent keeps its context.

## Things apps usually need next

- **Resume a stream** after a disconnect: the stream resumes from the last
  event id on its own; persist that id if your process restarts.
- **Client-side tools**: when the agent requests a tool your app implements, a
  `tool.call_requested` event arrives. Run the tool and send the result with
  `messages.create_tool_results` (Python) or `messages.createToolResults`
  (TypeScript). The session waits in `waiting_for_tool_results` until then.
- **Files**: the workspace files sub-client (`workspace_files` in Python)
  reads and writes workspace files: inputs for the agent, outputs it wrote.
- **Cost limits**: create a budget for the session before sending work.
- **Errors**: Python and TypeScript raise `AuthenticationError`,
  `NotFoundError`, `RateLimitError` and `ApiError`; Rust returns
  `Error::Auth` and `Error::Api { status, .. }`.

Method names beyond these differ slightly per language: check the package
README or the type hints in the editor instead of guessing.

## Read more

- SDK guide: <https://everruns.com/docs/features/sdk/>
- Event types: <https://everruns.com/docs/event-reference/>
- Streaming patterns: <https://everruns.com/docs/how-to/stream-events/>
- API reference and OpenAPI: <https://everruns.com/docs/api/>
- Source and examples: <https://github.com/everruns/sdk>
