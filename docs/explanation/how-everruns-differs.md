---
title: How Everruns differs from other agent runtimes
description: Where Everruns sits next to other agent runtimes and frameworks, what it shares with them, where it differs, and when another tool is the better fit.
sidebar:
  order: 4
---

People often ask how Everruns compares with other ways to run agents. This page
answers that plainly. It describes design choices, not winners: several of these
tools are good choices for problems Everruns does not try to solve.

Facts about other projects were checked against their own documentation in July
2026 and can change; follow the links for the current state.

## What Everruns is

- **A Rust framework and a durable runtime, MIT licensed end to end.** The
  `everruns` crate runs agents inside your own binary. The platform adds a
  control plane, stateless workers, a remote API and a web console on your own
  PostgreSQL. There is no paid edition and no feature held back.
- **Durable by default.** Every step of a turn is stored, so sessions survive
  restarts, deploys and lost workers, with no wall-clock ceiling. See
  [Durable execution](/explanation/durable-execution/).
- **Provider neutral.** OpenAI, Anthropic, Gemini, Bedrock, OpenRouter and
  others, plus your own driver. See [Providers](/providers/).
- **A community project.** A free, non-commercial project by [Mykhailo Chalyi](https://chaliy.name)
  and contributors, pre-1.0, so APIs still change.

## Hosted agent services

Examples: [Claude Managed Agents](https://platform.claude.com/docs/en/managed-agents/overview).

These run the agent harness for you on the vendor's infrastructure. Sessions
are stateful and resume after pauses, and you operate nothing.

- **Where it differs:** Everruns runs on your infrastructure and keeps session
  state in your database. A hosted service keeps state with the vendor and is
  usually tied to that vendor's models.
- **Choose the hosted service when** you are committed to one model vendor and
  want no infrastructure at all.

## Graph frameworks with a deployment platform

Examples: [LangGraph and LangSmith Deployment](https://www.langchain.com/langsmith/deployment).

The framework is open source and models an agent as a graph; a separate
deployment platform adds durable execution, an API and tracing. That platform
is proprietary, and self-hosting it is part of a commercial tier.

- **Where it differs:** the Everruns runtime and console are open source and
  self-hostable without a license tier. Everruns models a session as a
  reason-act loop with tools rather than an explicit graph.
- **Choose a graph framework when** you want explicit control over each step as
  a graph, or you already build on LangChain.

## TypeScript agent frameworks

Examples: [Mastra](https://mastra.ai).

A TypeScript framework with workflows, memory and model routing, deployed
mostly on serverless platforms. Workflows suspend and resume, and session
length follows the hosting platform's limits.

- **Where it differs:** Everruns is Rust, runs as long-lived workers on
  PostgreSQL, and ships a multi-tenant control plane with organizations,
  permissions, budgets and a console instead of leaving them to the
  application.
- **Choose a TypeScript framework when** your application is TypeScript and
  serverless, and agents fit inside a request or a short workflow.

## General durable execution engines

Examples: Temporal, Restate, Inngest.

These make arbitrary code durable. You can build an agent on them, and you
write the agent loop, tool handling, model calls and the UI yourself.

- **Where it differs:** Everruns is narrower. It is an agent runtime with the
  loop, tools, sessions, channels and a console built in.
- **Choose a general engine when** agents are a small part of a larger set of
  durable workflows you already run on it.

## Talk to us

If a comparison here is wrong or out of date, tell us on
[Discord](https://everruns.com/discord) or open an issue.
