# Tool Calling Tests

Tests for agent tool calling functionality via the API.

## Prerequisites

- API running at `http://localhost:9000`
- Temporal worker running
- Test capabilities available: `test_math`, `test_weather`

## Available Test Capabilities

| Capability | Tools | Description |
|------------|-------|-------------|
| `test_math` | add, subtract, multiply, divide | Calculator tools for math operations |
| `test_weather` | get_weather, get_forecast | Mock weather data tools |
| `web_fetch` | web_fetch | Fetch URLs and convert HTML to markdown/text |

## Running Tests

Run all tool calling tests automatically:

```bash
./scripts/tool-calling-tests.sh
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
./scripts/tool-calling-tests.sh --verbose

# Keep test agents for debugging
./scripts/tool-calling-tests.sh --skip-cleanup --verbose
```

## Test Coverage

| Test | Description |
|------|-------------|
| API Health Check | Verify API is available |
| Single Tool (TestMath Add) | Basic tool call with `add` |
| Multiple Tools (TestMath Operations) | Sequential `subtract` then `multiply` |
| TestWeather Tools (Multi-step) | Both `get_weather` and `get_forecast` |
| Parallel Tool Execution | Multiple `get_weather` calls in parallel |
| Combined Capabilities | Using both `test_math` and `test_weather` |
| Tool Error Handling | Division by zero error response |
| WebFetch Capability Available | Verify `web_fetch` capability is registered |
| Capability Detail Endpoint | Verify capability details include `system_prompt` and `tool_definitions` |
| WebFetch Tool (Input Validation) | Test `web_fetch` tool handles invalid URLs |

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
