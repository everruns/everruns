# TC003: Global Search - Edge Cases and Robustness

## Description

Verify that search handles adversarial and unusual inputs without crashing or performance degradation: poems, special characters, SQL wildcards, unicode, emoji, very long queries.

## Preconditions

- Control-plane running
- At least 1 agent exists

## Steps

1. Search with a poem (long query, many tokens):
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=Roses+are+red+violets+are+blue+sugar+is+sweet+and+so+are+you+the+sky+is+wide+the+ocean+deep+these+memories+I+shall+forever+keep"
   ```

2. Search with SQL LIKE wildcards (`%`, `_`):
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=%25_%5C"
   ```

3. Search with SQL injection attempt:
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=';+DROP+TABLE+agents;+--"
   ```

4. Search with unicode:
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=%E6%97%A5%E6%9C%AC%E8%AA%9E"
   ```

5. Search with emoji:
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=%F0%9F%A4%96"
   ```

6. Search with excessive whitespace:
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=+++hello+++world+++"
   ```

## Expected Results

- All steps: Server returns 200 with valid JSON response (empty results array is fine)
- Step 1: Response time < 500ms (tokens capped at 8, no query amplification)
- Step 2: LIKE wildcards treated as literal characters, not SQL wildcards
- Step 3: No SQL injection — parameterized queries prevent it
- Step 6: Collapsed to 2 tokens ("hello", "world")
