# Business Specification

## Abstract

Everruns is a durable AI agent execution platform that solves the fundamental problem of AI agent reliability. While other platforms focus on making agents smarter, Everruns makes them **unkillable**. This specification outlines business use cases and value propositions.

## Value Proposition

Traditional AI agents fail silently, lose context on crashes, and require manual restarts. Everruns provides:

- **Durability**: Agents survive infrastructure failures, deployments, and network issues
- **Resumability**: Execution state persists across restarts - resume from where you left off
- **Observability**: Real-time streaming via AG-UI protocol with full event history
- **Simplicity**: Configure, start, wait for results. Infrastructure reliability is handled.

## Market Context

AI agents are becoming capable of autonomous work spanning hours or days. Research from [METR](https://metr.org/blog/2025-03-19-measuring-ai-ability-to-complete-long-tasks/) shows task completion capabilities doubling every 7 months. Week-long autonomous tasks are expected within 2-4 years.

But long-running execution introduces infrastructure challenges:

- Host machines crash or restart
- Network connections drop
- External APIs hit rate limits or have outages
- Memory limits get exceeded
- LLM providers experience downtime

When a task runs for 6 hours and fails at hour 5, you lose all that work. Current agent frameworks assume reliable infrastructure that doesn't exist in practice.

### Industry Solutions

Current solutions focus on making agents smarter within sessions:

- **Anthropic's multi-session harness** ([blog post](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)): Addresses context window limitations with initializer and coding agents that maintain progress across sessions
- **OpenAI's Deep Research** ([announcement](https://openai.com/index/introducing-deep-research/)): Handles multi-step web research

These solve intelligence and memory problems. **Infrastructure reliability is different.** When the underlying compute fails, session-level solutions don't help. Durable execution requires workflow orchestration at the infrastructure level.

## Use Cases

### 1. Long-Running Research Agents

**Problem**: Research tasks that take hours or days fail when:
- Cloud instances restart
- Network connections drop
- Rate limits trigger retries
- Human oversight is needed mid-task

**Solution**: Configure an agent, start execution, wait for results. Infrastructure reliability is handled.

Everruns is built on [Temporal](https://temporal.io/) for durable execution. Every step is persisted, so agents resume from where they left off after any failure.

**Workflow**:
1. **Configure** — Define your agent: model, system prompt, tools, constraints
2. **Start** — Fire off the execution. Everruns handles the rest
3. **Monitor** — Real-time streaming via AG-UI protocol. Watch progress or check back later
4. **Survive failures** — Crashes, restarts, timeouts, API outages - execution continues from the last checkpoint

**Example Scenarios**:
- **Literature review** — Agent searches, reads, and synthesizes papers over several hours
- **Competitive analysis** — Agent monitors and compiles data from multiple sources over days
- **Code migration** — Agent refactors a large codebase incrementally, surviving machine restarts
- **Data processing** — Agent processes large datasets with external API calls that may rate-limit or fail

**Requirements**:
- Checkpoint every tool call result
- Resume from last successful step on failure
- Stream incremental findings as discovered
- Support human-in-the-loop approval gates

---

### 2. Multi-Day Data Processing Pipelines

**Problem**: Data pipelines involving AI decisions (classification, extraction, enrichment) need to:
- Process millions of records over days
- Handle API rate limits gracefully
- Recover from partial failures
- Maintain audit trails

**Solution**: Everruns provides durable execution with automatic retry, backoff, and progress tracking.

**Example Scenarios**:
- Document classification across large archives
- Entity extraction from unstructured data lakes
- Content moderation at scale
- Data quality assessment and correction

**Requirements**:
- Batch processing with configurable parallelism
- Automatic retry with exponential backoff
- Progress visibility and ETA estimation
- Partial result availability during execution

---

### 3. Autonomous Code Agents

**Problem**: AI coding assistants that work on multi-hour tasks need to:
- Survive IDE restarts and computer sleep
- Maintain context across long refactoring sessions
- Handle test/build cycles that take minutes
- Coordinate with CI/CD pipelines

**Solution**: Everruns decouples agent execution from client sessions, allowing work to continue in the background.

**Example Scenarios**:
- Large-scale codebase migrations
- Automated test generation across repositories
- Dependency upgrade campaigns
- Security vulnerability remediation

**Requirements**:
- Background execution independent of client connection
- Progress streaming when client reconnects
- Integration with external build systems
- Timeout handling for long tool executions

---

### 4. Customer Support Escalation Agents

**Problem**: Complex support cases require:
- Multi-step investigation across systems
- Waiting for external API responses
- Human approval before actions
- Handoffs between shifts

**Solution**: Everruns agents maintain case context indefinitely and support asynchronous human interaction.

**Example Scenarios**:
- Billing dispute investigation
- Technical issue diagnosis with log analysis
- Account recovery workflows
- Compliance request handling

**Requirements**:
- Persistent conversation context
- Pause/resume for human review
- External system integration
- SLA-aware prioritization

---

### 5. Workflow Orchestration Agents

**Problem**: Business workflows that span days or weeks need AI coordination:
- Multi-department approval chains
- Document generation and review cycles
- Regulatory compliance processes
- Contract negotiation assistance

**Solution**: Everruns provides durable workflow execution with event-driven progress.

**Example Scenarios**:
- Procurement approval workflows
- Employee onboarding automation
- Audit preparation and evidence gathering
- Vendor evaluation processes

**Requirements**:
- Timer-based scheduling (wait for events)
- External webhook triggers
- State machine execution
- Audit logging for compliance

---

### 6. Continuous Monitoring Agents

**Problem**: Always-on monitoring agents need to:
- Run indefinitely without degradation
- React to events in real-time
- Maintain historical context
- Scale with monitored systems

**Solution**: Everruns supports long-lived agents with efficient resource utilization.

**Example Scenarios**:
- Security anomaly detection
- Infrastructure health monitoring
- Social media sentiment tracking
- Market condition alerting

**Requirements**:
- Efficient idle state handling
- Event-triggered activation
- Resource cleanup on agent termination
- Multi-tenant isolation

---

## Platform Requirements

Based on use cases, the platform must support:

### Execution Model
1. Checkpointing at tool call boundaries
2. Automatic retry with configurable backoff
3. Timeout handling for long operations
4. Graceful cancellation

### State Management
1. Durable conversation history
2. Agent configuration versioning
3. Run state persistence
4. Event replay capability

### Integration
1. AG-UI protocol for UI compatibility
2. Webhook support for external triggers
3. API-first design for embedding
4. MCP tool protocol support (future)

### Observability
1. Real-time event streaming
2. Run status dashboards
3. Performance metrics
4. Cost tracking per agent/run

### Multi-tenancy
1. Workspace isolation
2. Per-tenant resource limits
3. Shared vs dedicated execution
4. Data residency controls

## Success Metrics

| Metric | Target | Description |
|--------|--------|-------------|
| Run Success Rate | >99% | Runs that complete without unrecoverable failure |
| Resume Latency | <5s | Time to resume from checkpoint after failure |
| Event Delivery | 100% | Events delivered to connected clients |
| Uptime | 99.9% | Platform availability |

## Competitive Positioning

| Feature | Everruns | Typical AI Platforms |
|---------|----------|---------------------|
| Run Duration | Hours to weeks | Minutes |
| Failure Recovery | Automatic resume | Manual restart |
| State Persistence | Full event history | Lost on crash |
| Background Execution | Native | Requires workarounds |
| Client Independence | Decoupled | Tight coupling |

## References

- [Measuring AI Ability to Complete Long Tasks](https://metr.org/blog/2025-03-19-measuring-ai-ability-to-complete-long-tasks/) — METR research on task duration trends
- [Effective Harnesses for Long-Running Agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) — Anthropic's multi-session approach
- [Introducing Deep Research](https://openai.com/index/introducing-deep-research/) — OpenAI's long-running research agent
