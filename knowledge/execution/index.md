# APIs and execution

* [API Specification](apis.md) - HTTP API endpoints, error handling.
* [API Conventions](api-conventions.md) - Cross-cutting HTTP API conventions.
* [Streaming APIs](api-streaming.md) - SSE streaming conventions for API endpoints.
* [Per-operation request/response examples](api-examples.md) - Per-operation request/response examples on `#[utoipa::path]` handlers.
* [LLM-specific OpenAPI extensions (`x-llm-*`)](api-llm-extensions.md) - LLM-specific OpenAPI extensions (`x-llm-*`).
* [SDK OpenAPI extensions (`x-sdk-*`)](api-sdk-extensions.md) - SDK model semantics in OpenAPI (`x-sdk-*`).
* [Public Endpoints](public-endpoints.md) - Public endpoints, error sanitization contract, stable public code set.
* [Error Disclosure](error-disclosure.md) - Semantic driver error kinds and session error-disclosure modes.
* [Events](events.md) - Event types, SSE streaming, contract and compatibility guarantees.
* [Execution Phases Specification](execution-phases.md) - Execution phases (Commentary/FinalAnswer) for multi-step tool flows.
* [Tool Execution Specification](tool-execution.md) - Tool types and execution flow.
* [Tool narration](tool-narration.md) - Backend-authored, argument-aware narration for common tool families.
* [Tool Output Distillation](tool-output-distillation.md) - Content-aware distillation of large non-exec tool results at capture time.
* [Capabilities Specification](capabilities.md) - Agent capabilities system.
* [Ask User](ask-user.md) - Structured choice questions that pause through the client-side tool-results lifecycle.
* [Soft Approval](soft-approval.md) - Spoken-consent confirmation before critical actions, as prompt guidance rather than a permission gate.
* [Tool Approval](tool-approval.md) - The hard per-call approval gate, and how hosted sessions park a turn on it durably until a person answers.
* [Browser Use](browser-use.md) - Provider-neutral `browser` tool: accessibility tree with element refs, form filling and tabs on a Browserless browser.
* [Computer Use](computer-use.md) - Provider-neutral computer use: screenshots plus pointer and keyboard actions on a display.
* [Guardrails Specification](guardrails.md) - Guardrails (capability-based output/tool-call checks).
* [Background Execution Capability](background-execution.md) - `background_execution` capability and cross-cutting / auto-activation contract.
* [Client-Side Tools](client-side-tools.md) - Client-side tools for API/SDK consumers.
* [Tool Search Specification](tool-search.md) - OpenAI tool_search deferred tool loading capability.
* [fetchkit](fetchkit.md) - fetchkit library powering the `web_fetch` capability.
* [Toolkit Library Contract](toolkit-library-contract.md) - Convention for external toolkit libraries.
* [Command Tree Specification](command-tree.md) - The `everruns <noun> <verb>` surface shared by scripted MCP, session shells, and Framework hosts.
* [Change Reasons and Manager Context](change-reasons-and-manager-context.md) - Every entity change records who, through what and why in a generic history with revisions and restore, which replaced agent versions, and each managed entity carries manager-only notes its own runtime never sees.
* [Bashkit Requirements for Custom FileSystem Adapters](bashkit-requirements.md) - Bash sandbox capabilities and requirements.
* [Lua Execution Capability (experimental)](lua-execution.md) - Experimental Lua execution capability (sandboxed VFS scripting; aims to supersede bashkit_shell).
* [OpenAI Steering Prototype](openai-steering-prototype.md) - Owned WebSocket experiment, durable updates, and recovery tradeoffs.
* [OpenAI Agents API Runtime Backend](openai-agents-api-runtime.md) - Mapping, control boundaries, durability, and recommendation for a managed OpenAI loop.
* [Native asynchronous tool calls](native-async-tools.md) - Custom-host pending-call coordination and recovery boundaries.
