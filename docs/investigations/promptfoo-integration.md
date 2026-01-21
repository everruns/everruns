# Promptfoo Integration Investigation

## Overview

[Promptfoo](https://www.promptfoo.dev/) is an open-source LLM testing and evaluation tool that enables systematic prompt engineering, regression testing, and red-teaming of LLM applications. This document investigates how Promptfoo can be used with Everruns.

## What Promptfoo Provides

### Core Capabilities

1. **Prompt Evaluation** - Compare different prompts/models side-by-side
2. **Regression Testing** - Catch regressions when prompts or models change
3. **CI/CD Integration** - Run evaluations in automated pipelines
4. **Red Teaming** - Security testing for LLM vulnerabilities (jailbreaks, prompt injection)
5. **Agent Evaluation** - Test agentic systems with multi-step behaviors
6. **Model Comparison** - Benchmark GPT vs Claude vs other providers

### Key Features

- Declarative YAML configuration (no code required)
- 50+ built-in LLM providers
- Custom HTTP provider for any API
- Assertion framework (contains, equals, similarity, LLM-as-judge)
- Token usage and cost tracking
- Latency measurement
- Local execution (no data leaves your machine)

## Integration Approaches for Everruns

### Approach 1: Direct LLM Provider Testing (Recommended Starting Point)

Test the underlying LLM providers directly, bypassing Everruns API.

```yaml
# promptfoo.yaml
description: "Everruns LLM Provider Evaluation"

providers:
  - id: openai:gpt-4o
  - id: anthropic:messages:claude-sonnet-4-20250514

prompts:
  - "You are a helpful assistant. {{task}}"
  - "You are a research assistant with access to web tools. {{task}}"

tests:
  - vars:
      task: "Explain quantum computing in simple terms"
    assert:
      - type: contains
        value: "qubit"
      - type: llm-rubric
        value: "Response is educational and accurate"

  - vars:
      task: "Write a Python function to calculate fibonacci"
    assert:
      - type: contains
        value: "def "
      - type: python
        value: |
          import ast
          try:
              ast.parse(output)
              return True
          except SyntaxError:
              return False
```

**Pros:**
- Simple setup, immediate value
- Tests prompt quality independent of Everruns infrastructure
- Useful for comparing models before deploying agents

**Cons:**
- Doesn't test Everruns-specific features (capabilities, tool execution)

### Approach 2: HTTP Provider for Everruns API

Test the full Everruns agent flow via HTTP provider.

```yaml
# promptfoo.yaml
description: "Everruns Agent E2E Evaluation"

providers:
  - id: https
    label: "Everruns Agent"
    config:
      # Step 1: Create session and send message
      url: "http://localhost:9000/v1/orgs/{{env.ORG_ID}}/agents/{{env.AGENT_ID}}/sessions"
      method: POST
      headers:
        Content-Type: "application/json"
        Authorization: "Bearer {{env.EVERRUNS_API_KEY}}"
      body:
        name: "Promptfoo Test Session"
      # Response transformation would need custom JS to:
      # 1. Extract session_id
      # 2. POST message to session
      # 3. Poll/stream for completion
      # 4. Return final assistant message
      transformResponse: "file://scripts/everruns-transform.js"

tests:
  - vars:
      prompt: "What is 2 + 2?"
    assert:
      - type: contains
        value: "4"
```

**Challenge:** Everruns uses async workflow execution with SSE streaming. A custom transform script is needed to:

```javascript
// scripts/everruns-transform.js
module.exports = async function({ response, vars, provider }) {
  const sessionId = response.id;
  const baseUrl = process.env.EVERRUNS_BASE_URL || 'http://localhost:9000';
  const orgId = process.env.ORG_ID;
  const agentId = process.env.AGENT_ID;

  // Send message to session
  const msgResponse = await fetch(
    `${baseUrl}/v1/orgs/${orgId}/agents/${agentId}/sessions/${sessionId}/messages`,
    {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${process.env.EVERRUNS_API_KEY}`
      },
      body: JSON.stringify({
        message: { content: [{ type: 'text', text: vars.prompt }] }
      })
    }
  );

  // Poll for completion (simplified - real impl needs SSE or polling)
  await new Promise(r => setTimeout(r, 5000));

  // Fetch messages
  const messagesResponse = await fetch(
    `${baseUrl}/v1/orgs/${orgId}/agents/${agentId}/sessions/${sessionId}/messages`,
    {
      headers: { 'Authorization': `Bearer ${process.env.EVERRUNS_API_KEY}` }
    }
  );
  const messages = await messagesResponse.json();

  // Return last assistant message
  const assistantMessages = messages.data.filter(m => m.role === 'assistant');
  const lastMessage = assistantMessages[assistantMessages.length - 1];
  return lastMessage?.content?.[0]?.text || '';
};
```

**Pros:**
- Tests full agent behavior including capabilities and tools
- Validates end-to-end system behavior

**Cons:**
- Complex setup with async handling
- Slower tests (full workflow execution)
- Requires running Everruns infrastructure

### Approach 3: Custom JavaScript Provider

More control with a dedicated provider file:

```yaml
# promptfoo.yaml
providers:
  - file://providers/everruns-provider.js

defaultTest:
  options:
    provider:
      config:
        agentId: "{{env.AGENT_ID}}"
        orgId: "{{env.ORG_ID}}"
        timeoutMs: 30000
```

```javascript
// providers/everruns-provider.js
const EventSource = require('eventsource');

class EverrunsProvider {
  constructor(options) {
    this.config = options.config || {};
    this.baseUrl = process.env.EVERRUNS_BASE_URL || 'http://localhost:9000';
  }

  id() {
    return 'everruns-agent';
  }

  async callApi(prompt, context) {
    const { agentId, orgId, timeoutMs = 30000 } = this.config;

    // Create session
    const session = await this.createSession(orgId, agentId);

    // Send message and wait for completion via SSE
    const response = await this.sendMessageAndWait(
      orgId, agentId, session.id, prompt, timeoutMs
    );

    return {
      output: response.text,
      tokenUsage: {
        prompt: response.promptTokens,
        completion: response.completionTokens,
        total: response.totalTokens
      },
      cost: response.cost
    };
  }

  // ... implementation details
}

module.exports = EverrunsProvider;
```

## Recommended Use Cases

### 1. System Prompt Evaluation

Compare different agent system prompts:

```yaml
prompts:
  - file://prompts/assistant-v1.txt
  - file://prompts/assistant-v2.txt

tests:
  - vars:
      task: "Summarize the key points of machine learning"
    assert:
      - type: llm-rubric
        value: "Response is clear, accurate, and well-organized"
      - type: cost
        threshold: 0.01  # Max $0.01 per call
```

### 2. Capability Impact Testing

Test how capabilities affect agent behavior:

```yaml
providers:
  - id: file://providers/everruns-provider.js
    label: "Agent without tools"
    config:
      agentId: "agent-no-tools"

  - id: file://providers/everruns-provider.js
    label: "Agent with web_fetch"
    config:
      agentId: "agent-with-web"

tests:
  - vars:
      task: "What is the current weather in San Francisco?"
    assert:
      - type: llm-rubric
        value: "Agent with tools provides current data; agent without acknowledges limitation"
```

### 3. Model Comparison

Compare different models on the same agent configuration:

```yaml
tests:
  - vars:
      model: "gpt-4o"
      task: "Write a SQL query to find top customers"
  - vars:
      model: "claude-sonnet-4-20250514"
      task: "Write a SQL query to find top customers"
    assert:
      - type: contains
        value: "SELECT"
      - type: latency
        threshold: 5000  # Max 5 seconds
```

### 4. Regression Testing in CI

```yaml
# .github/workflows/llm-eval.yml
name: LLM Evaluation

on:
  pull_request:
    paths:
      - 'prompts/**'
      - 'agents/**'

jobs:
  eval:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Promptfoo
        run: npm install -g promptfoo

      - name: Run evaluations
        run: promptfoo eval --ci
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
```

### 5. Red Teaming / Security Testing

```yaml
redteam:
  plugins:
    - prompt-injection
    - jailbreak
    - harmful
    - pii

  strategies:
    - basic
    - jailbreak
    - prompt-injection

providers:
  - openai:gpt-4o

prompts:
  - "You are a helpful assistant. User: {{query}}"
```

## Implementation Roadmap

### Phase 1: Immediate Value (No Code Changes)

1. **Install Promptfoo**: `npm install -g promptfoo`
2. **Create basic eval config** for testing system prompts
3. **Compare models** using direct provider testing
4. **Add to CI** for prompt regression testing

### Phase 2: Everruns Integration

1. **Create custom provider** (`providers/everruns-provider.js`)
2. **Handle SSE streaming** for async workflow completion
3. **Extract metrics** (tokens, latency, tool calls)
4. **Add agent-specific test cases**

### Phase 3: Advanced Evaluation

1. **Red teaming** for security vulnerabilities
2. **Capability matrix testing** (all capability combinations)
3. **Multi-turn conversation testing**
4. **Tool execution accuracy evaluation**

## Assertion Types Reference

| Type | Description | Example |
|------|-------------|---------|
| `contains` | Output contains substring | `value: "hello"` |
| `equals` | Exact match | `value: "yes"` |
| `regex` | Regex pattern match | `value: "\\d{4}"` |
| `llm-rubric` | LLM grades output | `value: "Is helpful and accurate"` |
| `cost` | Cost threshold | `threshold: 0.01` |
| `latency` | Response time (ms) | `threshold: 5000` |
| `javascript` | Custom JS assertion | `value: "output.length > 10"` |
| `python` | Custom Python assertion | `value: "len(output) > 10"` |
| `contains-json` | Valid JSON in output | Validates JSON structure |
| `is-json` | Output is valid JSON | Strict JSON validation |

## Files to Create

```
everruns/
├── promptfoo/
│   ├── promptfoo.yaml           # Main config
│   ├── providers/
│   │   └── everruns-provider.js # Custom Everruns provider
│   ├── prompts/
│   │   └── *.txt               # System prompt variants
│   ├── tests/
│   │   ├── basic.yaml          # Basic functionality tests
│   │   ├── capabilities.yaml   # Capability-specific tests
│   │   └── regression.yaml     # Regression test suite
│   └── redteam.yaml            # Security testing config
```

## Conclusion

Promptfoo can provide significant value for Everruns in three key areas:

1. **Prompt Engineering** - Systematically improve agent system prompts
2. **Quality Assurance** - Catch regressions before deployment
3. **Security Testing** - Red team agents for vulnerabilities

The recommended starting point is Phase 1 (direct LLM testing) which requires no code changes and provides immediate value. Phase 2 integration can follow once basic patterns are established.

## References

- [Promptfoo Documentation](https://www.promptfoo.dev/docs/intro/)
- [HTTP Provider Guide](https://www.promptfoo.dev/docs/providers/http/)
- [Agent Evaluation Guide](https://www.promptfoo.dev/docs/guides/evaluate-coding-agents/)
- [Red Teaming Guide](https://www.promptfoo.dev/docs/red-team/)
- [GitHub Repository](https://github.com/promptfoo/promptfoo)
