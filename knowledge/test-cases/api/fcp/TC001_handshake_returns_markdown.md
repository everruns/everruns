---
type: Test Case
title: "TC001: FCP handshake (GET) returns Markdown"
description: "Verify that a `GET` on a published agent channel's FCP endpoint returns the auto-generated Markdown handshake describing how to talk to the endpoint."
tags:
  - everruns
  - test-case
  - api
  - fcp
---
# TC001: FCP handshake (GET) returns Markdown

## Description

Verify that a `GET` on a published agent channel's FCP endpoint returns the auto-generated
Markdown handshake describing how to talk to the endpoint.

## Preconditions

- API server running (`just start-dev`)
- A published agent channel with an enabled `fcp` channel

## Test Data

| Field           | Value                                |
| --------------- | ------------------------------------ |
| Agent name        | `Flights`                            |
| Agent description | `Search and book commercial flights` |
| Channel type    | `fcp`                                |
| `anonymous`     | `true`                               |

## Steps

1. Create an agent backed by `llmsim` (or another active provider/model):
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/platform/providers" \
     -H "Content-Type: application/json" \
     -d '{"name":"FCP llmsim","provider_type":"llmsim"}'
   ```
   Save the provider `id`. Then create a model on it and an agent named Flights
   with description Search and book commercial flights. Save its ID as `AGENT_ID`.

2. Create the channel:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/platform/agents/$AGENT_ID/channels" \
     -H "Content-Type: application/json" \
     -d '{
       "channel_type":"fcp",
       "channel_config":{"anonymous":true}
     }'
   ```
   Save `id` from response as `CHANNEL_ID`.

3. Publish the channel:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/platform/agents/$AGENT_ID/channels/$CHANNEL_ID/publish" \
     -H "Content-Type: application/json" -d '{}'
   ```

4. Issue the FCP handshake:
   ```bash
   curl -i "http://localhost:9300/api/v1/channels/$CHANNEL_ID/fcp"
   ```

## Expected Result

| Check                                         | Expected                                          |
| --------------------------------------------- | ------------------------------------------------- |
| HTTP status                                   | `200`                                             |
| `Content-Type`                                | `text/markdown; charset=utf-8`                    |
| Body contains agent name                        | `# Flights`                                       |
| Body contains agent description                 | `Search and book commercial flights`              |
| Body mentions POST + Content-Type             | `POST`, `application/json`                        |
| Body mentions session cookie name             | `fcp_session`                                     |
| Body does NOT mention `Authorization: Bearer` | (anonymous endpoints omit the auth instructions)  |

## Validation Commands

```bash
curl -s -o /tmp/fcp.md -w '%{http_code} %{content_type}\n' \
  "http://localhost:9300/api/v1/channels/$CHANNEL_ID/fcp"
# Expect: 200 text/markdown; charset=utf-8

grep -q '^# Flights$' /tmp/fcp.md
grep -q 'fcp_session' /tmp/fcp.md
grep -q 'application/json' /tmp/fcp.md
! grep -q 'Authorization: Bearer' /tmp/fcp.md
```
