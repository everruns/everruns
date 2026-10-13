---
type: Test Case
title: "TC003: Global Chat - Create 10 Random Agents"
description: "Verify that the global chat agent can create 10 distinct agents in a single conversation when asked."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC003: Global Chat - Create 10 Random Agents

## Description

Verify that the global chat agent can create 10 distinct agents in a single conversation when asked. Each agent should have a unique addressable name (slug) and optional display name, plus a system prompt.

## Preconditions

- Canonical stack running (see root `AGENTS.md`)
- User logged in
- LLM API keys configured
- Disposable org with none of the ten requested names present; record the initial agent IDs

## Test Data

| Field | Value |
|-------|-------|
| User Message | Create 10 agents with different purposes. Name them: Code Reviewer, Data Analyst, Writing Editor, Math Tutor, DevOps Helper, Security Auditor, API Designer, Test Writer, Debug Assistant, and Doc Generator. Give each a relevant system prompt. |

## Steps

1. Open a Platform Chat thread: go to `/chats/new`; the managed Platform Chat is already selected
2. Send the message from test data above
3. Confirm when the chat agent asks for confirmation
4. Wait for the agent to finish creating all 10 agents (may take 30-60 seconds)
5. Observe the response — should list all created agents with links

## Expected Result

| Check | Expected |
|-------|----------|
| All 10 created | Response confirms 10 agents created |
| Unique names | Each agent has a distinct slug (e.g. `code-reviewer`) and display name (e.g. "Code Reviewer") |
| Agent links | Response contains 10 clickable agent links |
| Agents list | Navigate to `/agents` — all 10 visible with display names shown prominently and slugs in monospace underneath |
| Each agent active | Each agent has `status: active` |
| System prompts | Each agent has a system prompt relevant to its name |

## Validation

Record all ten returned agent IDs and compare them with the initial list. Require ten distinct new IDs with the requested names and usable detail links; pre-existing or merely narrated agents cannot satisfy the check. Delete only these recorded test agents after dependent cases finish.
