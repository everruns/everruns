# Core

* [Concepts](concepts.md) - Core entities, relationships, and concept diagram.
* [Architecture Specification](architecture.md) - System architecture, crate structure, infrastructure.
* [Code Organization](code-organization.md) - Developer conventions: formatting, testing, error handling, UI patterns.
* [Data Models Specification](models.md) - Data models (Agent, Session, Message, etc.).
* [ID Schema Specification](id-schema.md) - Standardized prefixed ID format.
* [Domain Modules](domains.md) - Domain modules: Command trait, feature-oriented structure, MCP catalog generation.
* [Runtime Specification](runtime.md) - Supported low-level execution host contract.
* [Sans-IO Turn State](sans-io-turn-state.md) - Converging the two turn-loop implementations on one serializable state with pure transitions.
* [Embedding Specification](embedding.md) - Embedding contract and `HostComposition`.
* [Providers Specification](providers.md) - Providers domain model: drivers, services, providers, models, model profiles.
* [LLM Drivers Specification](llm-drivers.md) - LLM driver trait, provider implementations.
* [OpenAI Responses WebSocket Transport](openai-responses-websocket.md) - Opt-in WebSocket transport for the OpenAI Responses driver: wire contract, opt-in, SSE fallback, connection reuse.
* [CLI Specification](cli.md) - CLI specification.
