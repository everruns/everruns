# Everruns API Examples

This directory contains examples for using the Everruns API.

## Examples

### agent_api_example.ipynb

A Jupyter notebook demonstrating the core API workflow:

- Health check
- Creating an agent with a system prompt
- Creating a session (conversation)
- Sending messages to trigger the agentic loop
- Polling for session completion
- Retrieving conversation messages
- Streaming real-time events via SSE
- Cleanup (delete session, archive agent)

**Requirements:**

```bash
pip install requests sseclient-py jupyter
```

**Usage:**

1. Start the Everruns API server (default: `http://localhost:9000`)
2. Open the notebook:
   ```bash
   jupyter notebook examples/agent_api_example.ipynb
   ```
3. Run the cells sequentially

## API Reference

- Swagger UI: http://localhost:9000/swagger-ui/
- OpenAPI spec: http://localhost:9000/api-doc/openapi.json
