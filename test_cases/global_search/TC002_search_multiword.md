# TC002: Global Search - Multi-Word Tokenized Search

## Description

Verify that multi-word search requires all tokens to match (AND semantics) and matches across name + description fields.

## Preconditions

- Control-plane running
- At least 2 agents with overlapping keywords

## Steps

1. Create agents:
   ```bash
   curl -s -X POST http://localhost:9000/v1/agents \
     -H "Content-Type: application/json" \
     -d '{"name": "Customer Support Bot", "description": "Handles billing inquiries", "system_prompt": "help"}'
   curl -s -X POST http://localhost:9000/v1/agents \
     -H "Content-Type: application/json" \
     -d '{"name": "Customer Feedback Analyzer", "description": "Analyzes NPS scores", "system_prompt": "analyze"}'
   ```

2. Multi-word search "customer bot":
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=customer+bot"
   ```

3. Cross-field search "customer billing":
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=customer+billing"
   ```

4. No-match multi-word "customer missing":
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=customer+missing"
   ```

## Expected Results

- Step 2: Returns 1 agent (only "Customer Support Bot" has both "customer" and "bot")
- Step 3: Returns 1 agent ("Customer Support Bot" — "customer" in name, "billing" in description)
- Step 4: Returns 0 agents (no entity contains both tokens)
