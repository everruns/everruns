---
title: Everruns Cloud quickstart
description: Sign up for Everruns Cloud, use the built-in model provider with starter credit, create a token, and connect from the SDK, CLI, or an AI tool.
sidebar:
  label: Everruns Cloud
appliesTo: [cloud]
---

[Everruns Cloud](https://app.everruns.com) runs the Everruns Platform for you:
the API, the workers, the database, and the UI. You create agents and sessions;
Everruns operates the infrastructure. It includes a built-in model provider, so
you can run an agent before you have any provider API keys.

This page takes you from sign-up to a first agent turn, then explains how
credit works.

## 1. Sign up

Open [app.everruns.com](https://app.everruns.com) and sign in. Verify your email
address when asked. When you create your first organization, it receives **$5
of starter credit**.

The starter credit is granted once per user account, on that user's first
organization, and needs a verified email. Creating more organizations does not
grant more. Starter credit is spent before any credit you buy and expires after
90 days.

## 2. Use the built-in provider

New organizations come with the built-in model provider already set up. There
are no keys to paste: create an agent in the UI and start a session, and its
turns are charged against your organization's credit.

## 3. Create a personal access token

The SDKs, the CLI, and the API authenticate with a personal access token. In the
UI, open **Settings** > **Personal Access Tokens** and create one. Tokens start
with `evr_pat_` and are shown once, so copy it before you close the dialog.

A token belongs to your user account, not to one organization, and can reach
every organization you belong to. Revoke it from the same page.

## 4. Connect

Pick the client you want. All three talk to `https://app.everruns.com`.

### SDK

Install an [SDK](/features/sdk/) and set two environment variables:

```bash
pip install everruns-sdk
export EVERRUNS_API_KEY=evr_pat_...
export EVERRUNS_API_URL=https://app.everruns.com/api
```

Set `EVERRUNS_API_URL` explicitly. Then run one turn:

```python
import asyncio
from everruns_sdk import Everruns


async def main():
    client = Everruns()  # reads EVERRUNS_API_KEY and EVERRUNS_API_URL
    agent = await client.agents.apply_by_name(
        name="hello", system_prompt="You are a concise assistant."
    )
    session = await client.sessions.create(agent_id=agent.id)
    await client.messages.create(session.id, "Say hello in one sentence.")
    print(f"Session: {session.id}")
    await client.close()


asyncio.run(main())
```

The message starts a turn; the answer arrives as events. The
[SDK tutorial](/tutorials/building-agents-using-sdk/) shows how to stream them.

### CLI

The [CLI](/features/cli/) uses Everruns Cloud by default, so there is no URL to
configure:

```bash
everruns login                       # opens the browser to sign in
SESSION=$(everruns sessions create -q)
everruns chat --session "$SESSION" "Say hello in one sentence."
```

`everruns login` signs you in through the browser, creates a personal access
token for this machine, and asks which organization to use when you belong to
more than one. On a machine without a browser, run `everruns login --token` and
paste a token you created in step 3.

### AI tools

The `everruns` plugin connects Claude Code, Codex, and Cursor to Everruns Cloud
over MCP, at `https://app.everruns.com/mcp`. It signs in with OAuth on first
use, so it needs no token. See [Use in AI Tools](/getting-started/use-in-ai-tools/).

## Credit and the spend cap

Everruns Cloud is prepaid. There is no subscription.

- **Balance.** **Settings** > **Billing** shows the organization's available
  credit, how much of it is starter credit and how much was purchased, and how
  much is held by work that is still running. Every member of the organization
  can see it.
- **Top up.** Organization owners and admins buy more credit from the same page.
  Payment goes through Stripe Checkout, and the credit appears once Stripe
  confirms the payment.
- **Spend cap.** Each model call reserves enough credit to cover its worst case
  before it runs, and is charged for actual usage when it finishes. When the
  balance reaches zero, the built-in provider refuses new calls, so turns that
  use it stop until someone tops up. A call that was already running when the
  credit ran out is still charged in full, so the balance can end slightly
  below zero; the next call is refused.

This credit is separate from [budgets](/advanced/budgets/). A budget is a limit
you set on a session or an agent; credit is what pays for the built-in provider.

## Bring your own keys (optional)

You can add your own provider API keys under **Settings** > **LLM Providers**,
and point agents at those models. Calls through your own keys are billed by
that provider, not against your Everruns credit, and keep working when the
credit balance is zero.

## Next steps

- [Concepts](/getting-started/concepts/): agents, sessions, harnesses, and
  capabilities.
- [Build your first agent](/tutorials/building-agents-using-sdk/): a guided SDK
  lesson that streams the agent's answer.
- [Capabilities](/capabilities/): the tools you can give an agent.
- [Self-hosted with Docker Compose](/getting-started/docker-compose/): run the
  same Platform on your own infrastructure.
