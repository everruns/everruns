---
title: Orchestrate multi-agent pipelines
description: Chain multiple Everruns agents together by passing output from one session into another.
---

A common pattern is splitting work across specialised agents, a researcher gathers facts, a writer turns them into prose, an editor polishes the result. Each is a separate agent and session; the application chains them.

This is the simplest orchestration pattern: no shared state, no subagent spawning, just sequential calls.

## Pipeline skeleton

```python
import asyncio
from everruns_sdk import Everruns


async def run_pipeline(client: Everruns, topic: str) -> str:
    researcher = await client.agents.create(
        name="Researcher",
        system_prompt="Research the given topic thoroughly. Write detailed notes.",
        capabilities=["web_fetch", "session_file_system"],
    )

    research_session = await client.sessions.create(agent_id=researcher.id)
    await client.messages.create(research_session.id, f"Research {topic}")

    research_output = await collect_final_text(client, research_session.id)

    writer = await client.agents.create(
        name="Writer",
        system_prompt="Write clear, well-structured technical articles.",
    )
    writer_session = await client.sessions.create(agent_id=writer.id)
    await client.messages.create(
        writer_session.id,
        f"Write a blog post based on this research:\n\n{research_output}",
    )

    return await collect_final_text(client, writer_session.id)


async def collect_final_text(client: Everruns, session_id: str) -> str:
    final_text: str | None = None
    async for event in client.events.stream(session_id):
        if event.type == "output.message.completed":
            message = event.data.get("message", {})
            final_text = "\n".join(
                p["text"] for p in message.get("content", []) if p.get("type") == "text"
            )
        elif event.type == "turn.failed":
            raise RuntimeError(event.data.get("error", "turn failed"))
        elif event.type == "turn.cancelled":
            raise RuntimeError("turn cancelled")
        elif event.type == "turn.completed":
            break
    if not final_text:
        raise RuntimeError("turn completed without producing a final message")
    return final_text
```

## Cleanup

Each session and agent persists until you explicitly delete it. For ephemeral pipelines, clean up at the end:

```python
for sid in (research_session.id, writer_session.id):
    await client.sessions.delete(sid)
for aid in (researcher.id, writer.id):
    await client.agents.delete(aid)
```

For pipelines you'll re-run, *don't* recreate the agents, create them once, store the IDs, and reuse them.

## When to use subagents instead

If one agent needs to delegate to another *during a turn*, use the [Sub Agents capability](/capabilities/sub-agents/) instead of an application-level pipeline. Subagents run inside the parent session and emit `subagent.*` events that the parent agent receives as tool results.

Pick application-level pipelines when:

- The stages are clearly separated and you want independent observability per stage.
- Stages run at different cadences (e.g., scheduled research → on-demand writeup).
- You want to reuse intermediate output across multiple downstream agents.

Pick subagents when:

- The parent agent decides at runtime which subagent to call.
- The work feels like a single user request, not a pipeline.

## See also

- [Sub Agents capability](/capabilities/sub-agents/)
- [Stream events](/how-to/stream-events/)
