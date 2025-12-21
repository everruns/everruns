# Tool Calling Tests

These tests verify the agent tool calling functionality via the API. They require the API and worker to be running.

## Prerequisites

- API running at `http://localhost:9000`
- Temporal worker running
- Test capabilities available: `test_math`, `test_weather`

## Available Test Capabilities

| Capability | Tools | Description |
|------------|-------|-------------|
| `test_math` | add, subtract, multiply, divide | Calculator tools for math operations |
| `test_weather` | get_weather, get_forecast | Mock weather data tools |

## Manual Tests

### 1. Single Tool Test (TestMath - Add)

Create an agent with test_math capability and test a single tool call:

```bash
# Create agent with test_math capability
MATH_AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Math Agent",
    "system_prompt": "You are a math assistant. Use the add tool to add numbers.",
    "description": "Tests single tool calling"
  }')
MATH_AGENT_ID=$(echo $MATH_AGENT | jq -r '.id')

# Set test_math capability
curl -s -X PUT "http://localhost:9000/v1/agents/$MATH_AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["test_math"]}' | jq

# Create session
MATH_SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Single Tool Test"}')
MATH_SESSION_ID=$(echo $MATH_SESSION | jq -r '.id')

# Send message requiring tool use
curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "What is 5 plus 3?"}}'

# Wait for workflow completion
sleep 15

# Check for tool results in messages
curl -s "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_results != null) | .tool_results'
```

**Expected**: Tool result containing `"result": 8`

### 2. Multiple Tools Test (Math - Multiple Operations)

Test agent using multiple different tools in one conversation:

```bash
# Use the same math agent
curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Calculate 10 minus 4, then multiply the result by 2"}}'

# Wait for workflow
sleep 20

# Check for multiple tool calls
curl -s "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" | \
  jq '[.data[] | select(.tool_calls != null) | .tool_calls[]] | length'
```

**Expected**: Multiple tool calls (subtract and multiply)

### 3. Multi-Step Agent Test (TestWeather Forecast)

Test agent that makes multiple tool calls across iterations:

```bash
# Create agent with test_weather capability
WEATHER_AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Weather Agent",
    "system_prompt": "You are a weather assistant. Get weather and forecasts for locations.",
    "description": "Tests multi-step tool calling"
  }')
WEATHER_AGENT_ID=$(echo $WEATHER_AGENT | jq -r '.id')

# Set test_weather capability
curl -s -X PUT "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["test_weather"]}'

# Create session
WEATHER_SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Multi-Step Test"}')
WEATHER_SESSION_ID=$(echo $WEATHER_SESSION | jq -r '.id')

# Request that requires weather + forecast
curl -s -X POST "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Get the current weather in New York and also the 5-day forecast"}}'

# Wait for workflow
sleep 20

# Check for both tool calls
curl -s "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_calls != null) | .tool_calls[] | .name'
```

**Expected**: Both `get_weather` and `get_forecast` tool calls

### 4. Parallel Tool Execution Test (Multiple Cities)

Test agent executing multiple tool calls in parallel:

```bash
# Request weather for multiple cities (should trigger parallel execution)
curl -s -X POST "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Get the current weather for New York, London, and Tokyo at the same time"}}'

# Wait for workflow
sleep 25

# Check for parallel tool calls (same assistant message with multiple tool calls)
curl -s "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_calls != null) | select(.tool_calls | length > 1) | {msg_id: .id, tool_count: (.tool_calls | length)}'
```

**Expected**: Assistant message with multiple tool calls (3 get_weather calls)

### 5. Combined Capabilities Test (TestMath + TestWeather)

Test agent with multiple capabilities:

```bash
# Create agent with both capabilities
COMBO_AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Combo Agent",
    "system_prompt": "You are a helpful assistant with math and weather tools.",
    "description": "Tests multiple capabilities"
  }')
COMBO_AGENT_ID=$(echo $COMBO_AGENT | jq -r '.id')

# Set both capabilities
curl -s -X PUT "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["test_math", "test_weather"]}'

# Create session
COMBO_SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Combo Capability Test"}')
COMBO_SESSION_ID=$(echo $COMBO_SESSION | jq -r '.id')

# Request using both capability types
curl -s -X POST "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/sessions/$COMBO_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Get the temperature in Tokyo, then add 10 to it"}}'

# Wait for workflow
sleep 20

# Check for both tool types
curl -s "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/sessions/$COMBO_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_calls != null) | .tool_calls[] | .name' | sort | uniq
```

**Expected**: Both weather and math tool calls

### 6. Tool Error Handling Test (Division by Zero)

Test that tool errors are handled correctly:

```bash
# Request division by zero
curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Divide 10 by 0"}}'

# Wait for workflow
sleep 15

# Check for tool error in results
curl -s "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_results != null) | .tool_results[] | select(.error != null) | .error'
```

**Expected**: Tool error message about division by zero

## Automated Tests

Run all tool calling tests automatically:

```bash
./.claude/skills/smoke-test/scripts/tool-calling-tests.sh
```

### Options

| Option | Description |
|--------|-------------|
| `--api-url URL` | Custom API URL (default: http://localhost:9000) |
| `--verbose` | Show detailed output |
| `--skip-cleanup` | Don't delete test agents after tests |

### Examples

```bash
# Run with verbose output
./.claude/skills/smoke-test/scripts/tool-calling-tests.sh --verbose

# Run against different API URL
./.claude/skills/smoke-test/scripts/tool-calling-tests.sh --api-url http://localhost:9000

# Keep test agents for debugging
./.claude/skills/smoke-test/scripts/tool-calling-tests.sh --skip-cleanup --verbose
```

### Test Summary

The automated script runs these tests:

| Test | Description |
|------|-------------|
| API Health Check | Verify API is available |
| Single Tool (TestMath Add) | Basic tool call with `add` |
| Multiple Tools (TestMath Operations) | Sequential `add` then `multiply` |
| TestWeather Tools (Multi-step) | Both `get_weather` and `get_forecast` |
| Parallel Tool Execution | Multiple `get_weather` calls in parallel |
| Combined Capabilities | Using both `test_math` and `test_weather` |
| Tool Error Handling | Division by zero error response |

## Troubleshooting

### No tool calls in response

1. Check that the capability is set on the agent:
   ```bash
   curl -s "http://localhost:9000/v1/agents/$AGENT_ID" | jq '.capabilities'
   ```

2. Verify the worker is running and processing workflows:
   ```bash
   tail -50 /tmp/worker.log
   ```

### Tool calls but no results

1. Check the workflow completed:
   ```bash
   curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID" | jq '.status'
   ```

2. Look for errors in worker logs:
   ```bash
   grep -i error /tmp/worker.log | tail -20
   ```

### Unexpected tool behavior

1. Verify the correct capability is being used:
   - `test_math` for calculator tools
   - `test_weather` for weather tools

2. Check tool definitions in the API:
   ```bash
   curl -s http://localhost:9000/v1/capabilities | jq '.[] | select(.id == "test_math")'
   ```
