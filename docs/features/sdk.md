---
title: SDK
description: Official client libraries for Rust, Python, and TypeScript
---

The Everruns SDK provides official client libraries for building AI agent applications. SDKs are available for Rust, Python, and TypeScript with a consistent API across all platforms.

## Features

- **Consistent API** across Rust, Python, and TypeScript
- **Async/await patterns** throughout all SDKs
- **SSE streaming** with automatic reconnection
- **Typed models** generated from OpenAPI specifications
- **Sub-client organization** (agents, sessions, messages, events)

## Installation

### Rust

Requires Rust 1.70+

```bash
cargo add everruns-sdk
```

### Python

Requires Python 3.10+

```bash
pip install everruns-sdk
```

### TypeScript

Requires Node.js 18+

```bash
npm install @everruns/sdk
```

## Authentication

All SDKs use the `EVERRUNS_API_KEY` environment variable by default. You can also pass the API key explicitly when initializing the client.

### Rust

```rust
use everruns_sdk::Client;

// From environment variable
let client = Client::from_env()?;

// Explicit API key
let client = Client::new("your-api-key");
```

### Python

```python
from everruns_sdk import Client

# From environment variable
client = Client()

# Explicit API key
client = Client(api_key="your-api-key")
```

### TypeScript

```typescript
import { Client } from "@everruns/sdk";

// From environment variable
const client = new Client();

// Explicit API key
const client = new Client({ apiKey: "your-api-key" });
```

## Quick Start

### Create an Agent and Start a Session

#### Rust

```rust
use everruns_sdk::{Client, CreateAgentRequest, CreateSessionRequest, CreateMessageRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::from_env()?;

    // Create an agent
    let agent = client.agents().create(CreateAgentRequest {
        name: "my-assistant".to_string(),
        system_prompt: "You are a helpful assistant.".to_string(),
        ..Default::default()
    }).await?;

    // Create a session
    let session = client.sessions().create(CreateSessionRequest {
        agent_id: agent.id.clone(),
        ..Default::default()
    }).await?;

    // Send a message
    client.messages().create(CreateMessageRequest {
        session_id: session.id.clone(),
        content: "Hello! What can you help me with?".to_string(),
    }).await?;

    Ok(())
}
```

#### Python

```python
import asyncio
from everruns_sdk import Client

async def main():
    client = Client()

    # Create an agent
    agent = await client.agents.create(
        name="my-assistant",
        system_prompt="You are a helpful assistant."
    )

    # Create a session
    session = await client.sessions.create(agent_id=agent.id)

    # Send a message
    await client.messages.create(
        session_id=session.id,
        content="Hello! What can you help me with?"
    )

asyncio.run(main())
```

#### TypeScript

```typescript
import { Client } from "@everruns/sdk";

const client = new Client();

// Create an agent
const agent = await client.agents.create({
  name: "my-assistant",
  systemPrompt: "You are a helpful assistant.",
});

// Create a session
const session = await client.sessions.create({
  agentId: agent.id,
});

// Send a message
await client.messages.create({
  sessionId: session.id,
  content: "Hello! What can you help me with?",
});
```

### Stream Events

#### Rust

```rust
use everruns_sdk::{Client, Event};
use futures::StreamExt;

let client = Client::from_env()?;
let mut stream = client.events().stream(&session.id).await?;

while let Some(event) = stream.next().await {
    match event? {
        Event::OutputMessageDelta { delta, .. } => {
            print!("{}", delta);
        }
        Event::TurnCompleted { .. } => {
            println!("\n--- Turn completed ---");
            break;
        }
        _ => {}
    }
}
```

#### Python

```python
async for event in client.events.stream(session_id=session.id):
    if event.type == "output.message.delta":
        print(event.data.delta, end="", flush=True)
    elif event.type == "turn.completed":
        print("\n--- Turn completed ---")
        break
```

#### TypeScript

```typescript
const stream = client.events.stream({ sessionId: session.id });

for await (const event of stream) {
  if (event.type === "output.message.delta") {
    process.stdout.write(event.data.delta);
  } else if (event.type === "turn.completed") {
    console.log("\n--- Turn completed ---");
    break;
  }
}
```

## API Coverage

All SDKs provide access to the full Everruns API:

| Resource | Operations |
|----------|------------|
| **Agents** | Create, list, get, update, archive |
| **Sessions** | Create, list, get, update, delete, cancel |
| **Messages** | Create, list |
| **Events** | Poll, stream (SSE) |
| **Filesystem** | Session file operations |
| **Images** | Upload, retrieve |

## Error Handling

SDKs provide standardized error types for common API errors:

| Error Type | Description |
|------------|-------------|
| `AuthenticationError` | Invalid or missing API key |
| `NotFoundError` | Resource not found |
| `RateLimitError` | Rate limit exceeded |
| `ApiError` | General API error |

### Rust

```rust
use everruns_sdk::Error;

match client.agents().get("invalid-id").await {
    Ok(agent) => println!("Found: {}", agent.name),
    Err(Error::NotFound(msg)) => println!("Not found: {}", msg),
    Err(Error::Authentication(msg)) => println!("Auth error: {}", msg),
    Err(e) => println!("Other error: {}", e),
}
```

### Python

```python
from everruns_sdk import NotFoundError, AuthenticationError

try:
    agent = await client.agents.get("invalid-id")
except NotFoundError as e:
    print(f"Not found: {e}")
except AuthenticationError as e:
    print(f"Auth error: {e}")
```

### TypeScript

```typescript
import { NotFoundError, AuthenticationError } from "@everruns/sdk";

try {
  const agent = await client.agents.get("invalid-id");
} catch (e) {
  if (e instanceof NotFoundError) {
    console.log(`Not found: ${e.message}`);
  } else if (e instanceof AuthenticationError) {
    console.log(`Auth error: ${e.message}`);
  }
}
```

## Event Types

The SDKs support all event types from the Everruns API:

| Category | Events |
|----------|--------|
| **Input** | `input.message` |
| **Output** | `output.message.started`, `output.message.delta`, `output.message.completed` |
| **Turn** | `turn.started`, `turn.completed`, `turn.failed`, `turn.cancelled` |
| **Tool** | `tool.started`, `tool.completed` |

See the [Events documentation](/features/events) for detailed information about event streaming.

## Resources

- [GitHub Repository](https://github.com/everruns/sdk) - Source code and examples
- [API Reference](/api) - Full API documentation
- [Events Documentation](/features/events) - Event streaming details

## Overview Video

<iframe
  width="100%"
  height="400"
  src="https://www.youtube.com/embed/1FfGKUTzBzA"
  title="Everruns SDK Overview"
  frameborder="0"
  allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
  allowfullscreen
  style="border-radius: 8px; margin-top: 1rem;">
</iframe>
