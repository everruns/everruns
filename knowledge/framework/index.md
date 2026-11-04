# Framework

* [How Sessions Run](engine-overview.md) - One-page plain-language map of the agent, Engine and hosting layers, where sessions live and what survives a crash.
* [Purpose and Terminology](purpose-and-terminology.md) - Audience, public identity, crate relationships, and the Framework/Runtime/SDKs/Platform boundary.
* [Application API Boundaries](application-api.md) - Application-facing composition, promotion decisions, and low-level host boundaries.
* [Event Listeners and Observability](observability.md) - Engine-level push listener delivery, isolation, statistics, and shutdown semantics.
* [Provider Extension Boundary](provider-extension.md) - Why model identity and provider execution are open, separate concerns.
* [Framework Harnesses](harnesses.md) - Promoting the harness to an application-facing value, separating an agent's world from its behavior and its machine.
* [Library Experience](library-experience.md) - Ergonomic and behavioral success bars for application-facing use.
* [Documentation and Examples](documentation-and-examples.md) - The maintained public learning path and runnable-example contract.
* [API Stability](api-stability.md) - Stable versus alpha markers, promises, and the marking convention.
* [serve (experimental)](serve.md) - Why the experimental serve crates pair Topcoat's API shape with eve's hosting model, and what is still open.
* [Execution Backends](execution-backends.md) - Engine hosting profiles, shared actor/log recovery and remaining internal turn backends.
* [Engine-Owned Durable Sessions](engine-durable-sessions.md) - Engine/Session hosting boundary, durable admission handles, shared execution and native/DO/serve integration.
* [Local Authority Cutover](local-authority-cutover.md) - Lossless import of legacy local/custom-backend sessions into the SQLite authority, gap matrix and the legacy lane retirement gate.
* [Portable Durable Agent Execution](durable-execution-portability.md) - Draft architecture for shared runtime execution over native actors, Cloudflare Durable Objects and celld, with deployment alternatives and a phased recommendation.
* [Durable Execution Guarantees and Backend Qualification](durable-execution-guarantees.md) - Proposed admission, activation, fencing, operation recovery, wait and versioning contracts, with backend qualification scenarios.
* [The Execution Kernel in a JavaScript Isolate](wasm-kernel.md) - Why provider, core and engine build for wasm32, how time and tasks port, and the shared Framework Engine application used by Cloudflare and celld.
* [Voice Agents](voice-agents.md) - Accepted design (phase 1 built) for building and serving voice agents: voice as a channel next to text, one voice loop in core shared by the Framework, serve and the server, and speech drivers as provider services.
* [Shared Execution with Durable Object Authority](durable-object-native.md) - Native and Wasm profiles, shared actor driver, trust boundary and backend qualification.

* [Provider-Native Batch Requests](batch-model-requests.md) - Offline completion lifecycle, scoped tickets, provider boundary and uncertain submission recovery.
