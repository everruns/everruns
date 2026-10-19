---
type: Test Case
title: "TC005: Global Chat - Run All 10 Agents"
description: "Verify that the global chat agent can run all 10 previously created agents sequentially, each with a relevant task, and relay all results."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC005: Global Chat - Run All 10 Agents

## Description

Verify that the global chat agent can run all 10 previously created agents sequentially, each with a relevant task, and relay all results.

## Preconditions

- Canonical stack running (see root `AGENTS.md`)
- User logged in
- LLM API keys configured
- All 10 agents from TC003 exist

## Test Data

| Field | Value |
|-------|-------|
| User Message | Run each of these agents with a short test task: Code Reviewer ("Review this function: def add(a,b): return a+b"), Data Analyst ("What is the mean of [10, 20, 30]?"), Writing Editor ("Fix: 'Their going to the store'"), Math Tutor ("What is 15% of 200?"), DevOps Helper ("What port does HTTPS use?"), Security Auditor ("Is eval() safe in Python?"), API Designer ("Suggest a REST endpoint for user creation"), Test Writer ("Write a test for an add function"), Debug Assistant ("Why might a null pointer exception occur?"), Doc Generator ("Write a one-line docstring for a sort function"). |

## Steps

1. Open a Platform Chat thread: go to `/chats/new`; the managed Platform Chat is already selected
2. Send the message from test data above
3. Record pre-existing session IDs. Wait up to five minutes for the requested runs and record the new session/agent IDs from the tool trace and final result
4. Observe the response

## Expected Result

| Check | Expected |
|-------|----------|
| All 10 run | Chat agent reports results from all 10 agents |
| Sessions created | 10 new sessions appear at `/sessions` |
| Each agent responded | Each result contains a non-empty answer |
| Code Reviewer | Response references the `add` function |
| Data Analyst | Response mentions `20` (mean) |
| Writing Editor | Response corrects to "They're" |
| Math Tutor | Response mentions `30` |
| DevOps Helper | Response mentions `443` |
| Security Auditor | Response warns against `eval()` |
| API Designer | Response suggests a `POST /users` or similar |
| Test Writer | Response contains test code |
| Debug Assistant | Response explains null pointer causes |
| Doc Generator | Response contains a docstring |

## Validation

Compare the recorded new session IDs with the requested ten agent IDs. Require one completed task and relevant non-empty response per requested agent. Existing sessions cannot satisfy the assertion. Follow each returned run link into Trace and compare its task/response with the relayed result.
