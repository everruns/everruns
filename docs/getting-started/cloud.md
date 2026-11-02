---
title: Everruns Demo quickstart
description: Try the hosted demo with available promotional credit or your own provider keys, create a token, and connect from the SDK, CLI, or an AI tool.
sidebar:
  label: Everruns Demo
appliesTo: [cloud]
---

[Everruns Demo](https://app.everruns.com) runs the Everruns Platform for you:
the API, the workers, the database, and the UI. You create agents and sessions;
Everruns operates the infrastructure. Explore with available promotional
credit for built-in models, or connect your own provider keys.

This page takes you from sign-up to a first agent turn, then explains how
credit works.

## 1. Sign up

Open [app.everruns.com](https://app.everruns.com) and sign in. Verify your email
address when asked. Open **Settings** > **Billing** to see any available
promotional credit and its expiration. Credit purchases and top-ups are
unavailable.

## Manage your Everruns ID

Open **Settings** > **Profile** > **Manage your Everruns ID**, or visit
[id.everruns.com/account](https://id.everruns.com/account), to manage sign-in
methods, security, sign-in sessions, and personal connections across your
organizations.

Passkeys are an optional second factor after your password or Google sign-in.
In **Security**, choose **Add a passkey** and follow your browser's native
prompt. Compatible browsers can use Apple Passwords, a mobile credential
manager, a password manager, or a security key. Save the recovery codes shown
after enrollment; each code works once after primary sign-in. Keep a backup
passkey or recovery codes so losing a device does not lock you out.

**Sessions** shows signed-in devices, activity, and, for new sign-ins, masked IP
addresses and approximate sign-in locations. Ending an ID session leaves an
app's separate session valid until its own expiry. **Connections** lists your
ID-linked apps, approved AI clients, personal MCP servers, service connections,
credentials, and API tokens, with revoke controls and recorded usage. Unknown
usage is shown as unavailable.

## 2. Use the built-in provider

New organizations come with the built-in model provider already set up. There
are no keys to paste: create an agent in the UI and start a session, and its
turns are charged against your organization's credit.

## 3. Create a personal access token

The SDKs, the CLI, and the API authenticate with a personal access token. In the
UI, open **Settings** > **Personal Access Tokens** and create one. Tokens start
with `evr_pat_` and are shown once, so copy it before you close the dialog.

A token belongs to your user account, not to one organization, and can reach
every organization you belong to. Choose **Read-only** for scripts that only
read; it still reads every organization you belong to
([Token access](/features/ui/#token-access)). Revoke a token from the same page.

## 4. Connect

Pick the client you want. All three talk to `https://app.everruns.com`.

### SDK

Install an [SDK](/features/sdk/) and set two environment variables:

```bash
pip install everruns-sdk
export EVERRUNS_API_KEY=evr_pat_...
export EVERRUNS_API_URL=https://app.everruns.com/api
```

Newer SDK releases default to Everruns Demo; setting `EVERRUNS_API_URL` keeps
older ones pointed at it too. Then run one turn:

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

The [CLI](/features/cli/) uses Everruns Demo by default, so there is no URL to
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

The `everruns` plugin connects Claude Code, Codex, and Cursor to Everruns Demo
over MCP, at `https://app.everruns.com/mcp`. It signs in with OAuth on first
use, so it needs no token. See [Use in AI Tools](/getting-started/use-in-ai-tools/).

## Credit and the spend cap

The demo has no subscriptions, credit purchases, top-ups, or paid overage.

- **Balance.** **Settings** > **Billing** shows the organization's available
  credit, its expiration, and amounts held by work that is still running.
  Every organization member can see the balance. Historical purchased credit
  and refund records remain visible when present.
- **Built-in models.** Requests use the organization's available credit.
  When no credit is available, use your own provider keys or self-host
  Everruns to continue exploring. Work already in flight can still settle
  against its reservation.

This credit is separate from [budgets](/advanced/budgets/). A budget is a limit
you set on a session or an agent; credit is what pays for the built-in provider.

## Bring your own keys (optional)

You can add your own provider API keys under **Models** > **Providers**,
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
