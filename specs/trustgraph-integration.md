# TrustGraph Integration Analysis

Analysis of how ideas from [TrustGraph](https://github.com/trustgraph-ai/trustgraph) could apply to everruns.

## What TrustGraph's Context Graph Does

TrustGraph is an open-source Agent Intelligence Platform that replaces flat RAG (text chunks + vector similarity) with a **context graph** — a knowledge graph optimized for LLM consumption.

### Core Mechanism

1. **Ingestion**: Documents are chunked, then three LLM-powered agents extract in parallel:
   - Topics (themes, subjects)
   - Entities (people, systems, concepts)
   - Relationships (connections between entities)

2. **Storage**: Extracted knowledge is stored as **triples** (`Subject → Predicate → Object`) in a graph database (Cassandra, Neo4j, FalkorDB) with vector embeddings in Qdrant.

3. **Query-time subgraph extraction**: Instead of returning raw text chunks, TrustGraph:
   - Embeds the query into a vector
   - Finds semantically nearest entities via vector search
   - Traverses graph edges to expand into a **relevant subgraph**
   - Delivers that subgraph (with relevance scores and provenance) as structured context to the LLM

4. **Context Cores**: Portable packages bundling a knowledge graph + embeddings. Loadable/unloadable at runtime in seconds.

### Why This Matters vs. Traditional RAG

| Aspect | Text-chunk RAG | Context Graph |
|--------|---------------|---------------|
| Retrieved data | Flat text snippets | Connected entity subgraphs |
| Multi-hop reasoning | Fails (chunks are isolated) | Natural (follow graph edges) |
| Token efficiency | Redundant, verbose | 50-70% fewer tokens |
| Provenance | Weak | Source-attributed triples |
| Temporal reasoning | None | Temporal ordering on edges |

The key insight: LLMs reason better over structured relationships than over bags of text.

## Ideas Applicable to Everruns

### 1. Session Knowledge Graph (High Value)

**Problem in everruns**: Sessions accumulate context as a flat event log. Long sessions hit context limits. Compaction loses information.

**TrustGraph idea**: Build a **per-session knowledge graph** from events, tool results, and messages. As a session progresses, entities and relationships are extracted and stored as triples.

**How it would work in everruns**:
- After each turn, an extraction step identifies entities and relationships from messages and tool outputs
- Store triples in the session's PostgreSQL-backed storage (new `session_knowledge_graph` table, or use `session_sql_database` capability)
- On context compaction, instead of summarizing text, query the knowledge graph for a relevant subgraph based on the current user message
- The agent retains structured memory of everything it learned, even after compaction

**Integration point**: New capability (`session_knowledge_graph`) contributing a system prompt section with graph-derived context, updated after each turn.

**Architectural fit**: Aligns with everruns' capability system — this would be a new capability that adds system prompt context and message filters.

### 2. Cross-Session Knowledge Persistence (High Value)

**Problem**: Each everruns session starts from scratch. An agent that learned about a codebase in session 1 has no memory in session 2.

**TrustGraph idea**: **Context Cores** — portable, reusable knowledge packages.

**How it would work in everruns**:
- Extract a knowledge graph from a completed session
- Store it as an agent-level or harness-level "knowledge core"
- When a new session starts, load the relevant knowledge core as additional context
- Multiple cores could be composed (e.g., "codebase knowledge" + "user preferences" + "domain expertise")

**Integration point**: New entity in the data model (`KnowledgeCore`) associated with agents or harnesses. Loaded during session assembly as a capability contribution.

### 3. Capability-Aware Tool Selection via Graph (Medium Value)

**Problem**: Agents with many capabilities/tools face the "tool sprawl" problem — the LLM gets a long list of tools and sometimes picks wrong ones.

**TrustGraph idea**: Use graph relationships to surface only the most relevant tools for a given query.

**How it would work in everruns**:
- Build a graph of tool relationships, use-patterns, and domain associations
- At query time, use the user's message to retrieve only the most relevant tool subset
- Present a focused tool list to the LLM instead of the full capability set

**Integration point**: `CapabilityService` could apply a graph-based filter before assembling `RuntimeAgent` tools.

### 4. Document Knowledge for Agents (Medium Value)

**Problem**: Agents that need domain knowledge currently rely on system prompts (static) or `web_fetch` / MCP (per-query, no persistence).

**TrustGraph idea**: Ingest documents into a knowledge graph, query relevant subgraphs at inference time.

**How it would work in everruns**:
- New capability: `knowledge_base` — allows uploading documents to an agent
- Documents are processed through entity/relationship extraction (using the agent's own LLM)
- At query time, the relevant subgraph is injected into the system prompt
- Stored per-agent or per-harness, reusable across sessions

**Integration point**: A new capability that uses the session filesystem for document storage and PostgreSQL for the knowledge graph. Could integrate with `sample_data` mounts.

### 5. Event Stream as Knowledge Source (Medium Value)

**Problem**: Everruns has a rich event log but uses it primarily for message reconstruction and SSE streaming.

**TrustGraph idea**: Treat event streams as a source for knowledge extraction.

**How it would work in everruns**:
- Post-session analysis: extract patterns from event logs across sessions
- Build a meta-knowledge graph: which tools succeed for which tasks, common error patterns, effective prompts
- Use this for agent self-improvement recommendations

**Integration point**: Background job (scheduled task via durable engine) that processes completed sessions.

### 6. Ontology-Guided Extraction (Lower Value, Specialist Use)

**TrustGraph idea**: Use OWL ontologies to guide entity extraction for domain-specific precision.

**Applicability**: Useful if everruns agents operate in regulated/specialist domains (medical, legal, financial) where extraction must conform to a formal schema. Lower priority for general-purpose agent use.

## Recommended Implementation Order

1. **Session Knowledge Graph** — Most impactful. Solves the real problem of long-session context degradation. Fits naturally as a new capability.

2. **Cross-Session Knowledge Persistence** — Builds on (1). High value for agents that interact with the same domain repeatedly.

3. **Document Knowledge for Agents** — Independent from (1) and (2). Provides a cleaner alternative to stuffing system prompts with domain text.

4. **Tool Selection via Graph** — Becomes valuable when agents have 20+ tools. Can wait until tool sprawl is actually a problem.

5. **Event Stream Analysis** — Background improvement. No urgency.

## Key Architectural Considerations

### What to Adopt

- **Triple-based storage** for knowledge (Subject → Predicate → Object). Simple, flexible, queryable.
- **Dual retrieval** (vector similarity + graph traversal) for subgraph extraction. Much better than either alone.
- **Portable knowledge packages** (Context Cores). Separates expensive extraction from cheap loading.

### What NOT to Adopt

- **Apache Pulsar backbone**: Everruns already has a durable execution engine with PostgreSQL + gRPC. Adding Pulsar would be massive complexity for no gain.
- **Separate graph database**: For everruns' scale, PostgreSQL with a triples table + `pgvector` extension handles both graph storage and vector search. No need for Cassandra/Neo4j/Qdrant.
- **60+ CLI tools**: Everruns is API-first. Knowledge graph operations should be capabilities and API endpoints, not CLI tools.

### Minimal Implementation Sketch

```
New table: session_knowledge_triples
  - session_id (FK)
  - subject TEXT
  - predicate TEXT
  - object TEXT
  - source_event_id (FK, provenance)
  - embedding VECTOR(1536) (pgvector)
  - created_at TIMESTAMP

New capability: session_knowledge_graph
  - After each turn: extract entities/relationships from new events
  - Before each turn: query graph for relevant subgraph, inject into system prompt
  - Tool: "query_knowledge" — lets agent explicitly search its knowledge graph
```

This uses everruns' existing PostgreSQL infrastructure, capability system, and event model. No new infrastructure required.

## Summary

TrustGraph's core insight — that structured knowledge graphs produce better LLM context than flat text — directly addresses everruns' challenge of maintaining agent context over long and repeated sessions. The session knowledge graph capability is the highest-value integration, fitting cleanly into everruns' existing architecture without requiring new infrastructure.
