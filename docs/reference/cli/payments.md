---
title: everruns payments
description: "Machine payments: wallets, spend policies and attempts. CLI reference for everruns payments."
sidebar:
  label: payments
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Machine payments: wallets, spend policies and attempts.

| Command | What it does |
|---|---|
| [`payments accounts create`](#payments-accounts-create) | Create a machine-payment wallet account. |
| [`payments accounts disable`](#payments-accounts-disable) | Disable a machine-payment wallet account. |
| [`payments accounts get`](#payments-accounts-get) | Get a machine-payment wallet account. |
| [`payments accounts list`](#payments-accounts-list) | List machine-payment wallet accounts. |
| [`payments accounts update`](#payments-accounts-update) | Update a machine-payment wallet account. |
| [`payments attempts list`](#payments-attempts-list) | List machine-payment attempts. |
| [`payments policies create`](#payments-policies-create) | Create a machine-payment spend policy. |
| [`payments policies disable`](#payments-policies-disable) | Disable a machine-payment spend policy. |
| [`payments policies get`](#payments-policies-get) | Get a machine-payment spend policy. |
| [`payments policies list`](#payments-policies-list) | List machine-payment spend policies. |
| [`payments policies update`](#payments-policies-update) | Update a machine-payment spend policy. |

## payments accounts create

Create a machine-payment wallet account.

```bash
everruns payments accounts create [OPTIONS] --label <label> --owner-id <owner_id> --owner-type <owner_type> --rail <rail>
```

| Flag | Description |
|---|---|
| `--label <LABEL>` | Required. Human-readable label. |
| `--metadata <METADATA>` | Free-form metadata attached to this account (caller-defined; opaque to the platform). Example... |
| `--owner-id <OWNER_ID>` | Required. Prefixed identifier of the owning principal. |
| `--owner-type <OWNER_TYPE>` | Required. Principal class that owns the account. |
| `--private-key <PRIVATE_KEY>` | Private key material for the rail. |
| `--public-address <PUBLIC_ADDRESS>` | Public address on the rail (chain address, account number, etc.). |
| `--rail <RAIL>` | Required. Settlement rail this account operates on. |

Example:

```bash
# Register a wallet an agent can pay from
everruns payments accounts create --owner-type organization --owner-id org_01h9 --rail mpp_tempo --label 'Research wallet' --public-address 0x742d35Cc6634C0532925a3b844Bc454e4438f44e --reason 'Fund paid API calls'
```

## payments accounts disable

Disable a machine-payment wallet account.

```bash
everruns payments accounts disable [OPTIONS] --payment-account-id <payment_account_id>
```

| Flag | Description |
|---|---|
| `--payment-account-id <PAYMENT_ACCOUNT_ID>` | Required. Payment account's prefixed public identifier. |

Example:

```bash
# Stop all spending from a wallet
everruns payments accounts disable --payment-account-id payacct_01h9 --reason 'Wallet compromised'
```

## payments accounts get

Get a machine-payment wallet account.

```bash
everruns payments accounts get [OPTIONS] --payment-account-id <payment_account_id>
```

| Flag | Description |
|---|---|
| `--payment-account-id <PAYMENT_ACCOUNT_ID>` | Required. Payment account's prefixed public identifier. |

Example:

```bash
# Inspect a wallet's rail, owner and status
everruns payments accounts get --payment-account-id payacct_01h9
```

## payments accounts list

List machine-payment wallet accounts.

```bash
everruns payments accounts list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--owner-id <OWNER_ID>` | Only accounts owned by this principal's prefixed public identifier. |
| `--owner-type <OWNER_TYPE>` | Only accounts owned by this kind of principal: `user`, `virtual_user`, or `organization`. |

Example:

```bash
# List the wallets an organization owns
everruns payments accounts list --owner-type organization --owner-id org_01h9
```

## payments accounts update

Update a machine-payment wallet account.

```bash
everruns payments accounts update [OPTIONS] --payment-account-id <payment_account_id>
```

| Flag | Description |
|---|---|
| `--label <LABEL>` | New label. |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--payment-account-id <PAYMENT_ACCOUNT_ID>` | Required. Payment account's prefixed public identifier. |
| `--private-key <PRIVATE_KEY>` | New private key to rotate in. |
| `--public-address <PUBLIC_ADDRESS>` | New public address on the rail. |
| `--status <STATUS>` | Current lifecycle status. |

Example:

```bash
# Rotate a wallet's label or deactivate it
everruns payments accounts update --payment-account-id payacct_01h9 --label 'Research wallet (prod)' --reason 'Mark the production wallet'
```

## payments attempts list

List machine-payment attempts.

```bash
everruns payments attempts list [OPTIONS] --limit <limit>
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Required. Maximum number of items returned in this page. |
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Audit what a session tried to pay for
everruns payments attempts list --session-id session_01h9 --limit 20
```

## payments policies create

Create a machine-payment spend policy.

```bash
everruns payments policies create [OPTIONS] --payment-account-id <payment_account_id> --subject-id <subject_id> --subject-type <subject_type>
```

| Flag | Description |
|---|---|
| `--allowed-capabilities <ALLOWED_CAPABILITIES>` | Capability IDs this policy permits paid calls for. Repeatable. |
| `--allowed-hosts <ALLOWED_HOSTS>` | HTTP host allowlist for paid outbound calls. Repeatable. |
| `--max-amount-usd-per-day <MAX_AMOUNT_USD_PER_DAY>` | Maximum cumulative amount (USD) per UTC day. |
| `--max-amount-usd-per-request <MAX_AMOUNT_USD_PER_REQUEST>` | Maximum amount (USD) any single paid request may settle for. |
| `--max-amount-usd-per-turn <MAX_AMOUNT_USD_PER_TURN>` | Maximum cumulative amount (USD) per agent turn. |
| `--metadata <METADATA>` | Free-form metadata attached to this policy. Example: `{"owner_team": "ops", "ticket": "OPS-12... |
| `--payment-account-id <PAYMENT_ACCOUNT_ID>` | Required. Payment account this policy authorizes spending from. |
| `--rail-preference <RAIL_PREFERENCE>` | Preferred settlement rails in priority order; the authority picks the first available. Repeatable. |
| `--require-approval-above-usd <REQUIRE_APPROVAL_ABOVE_USD>` | Threshold (USD) above which a request would require explicit human approval. |
| `--subject-id <SUBJECT_ID>` | Required. Prefixed identifier of the bound subject. |
| `--subject-type <SUBJECT_TYPE>` | Required. Class of subject this policy binds to. |

Example:

```bash
# Limit what an agent may spend from a wallet
everruns payments policies create --payment-account-id payacct_01h9 --subject-type agent --subject-id agent_01h9 --allowed-hosts api.shippo.com --max-amount-usd-per-request 5 --reason 'Allow shipping quotes up to 5 USD'
```

## payments policies disable

Disable a machine-payment spend policy.

```bash
everruns payments policies disable [OPTIONS] --payment-policy-id <payment_policy_id>
```

| Flag | Description |
|---|---|
| `--payment-policy-id <PAYMENT_POLICY_ID>` | Required. Payment policy's prefixed public identifier. |

Example:

```bash
# Revoke an agent's permission to spend
everruns payments policies disable --payment-policy-id paypol_01h9 --reason 'Integration retired'
```

## payments policies get

Get a machine-payment spend policy.

```bash
everruns payments policies get [OPTIONS] --payment-policy-id <payment_policy_id>
```

| Flag | Description |
|---|---|
| `--payment-policy-id <PAYMENT_POLICY_ID>` | Required. Payment policy's prefixed public identifier. |

Example:

```bash
# Inspect a spend policy's limits and allowlists
everruns payments policies get --payment-policy-id paypol_01h9
```

## payments policies list

List machine-payment spend policies.

```bash
everruns payments policies list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--payment-account-id <PAYMENT_ACCOUNT_ID>` | Only policies that authorize spending from this payment account. |
| `--subject-id <SUBJECT_ID>` | Only policies bound to this subject's prefixed public identifier. |
| `--subject-type <SUBJECT_TYPE>` | Only policies bound to this kind of subject (`user`, `virtual_user`, `agent`, `agent_channel`... |

Example:

```bash
# See which policies authorize spending from a wallet
everruns payments policies list --payment-account-id payacct_01h9
```

## payments policies update

Update a machine-payment spend policy.

```bash
everruns payments policies update [OPTIONS] --payment-policy-id <payment_policy_id>
```

| Flag | Description |
|---|---|
| `--allowed-capabilities <ALLOWED_CAPABILITIES>` | Replacement capability allowlist. Repeatable. |
| `--allowed-hosts <ALLOWED_HOSTS>` | Replacement host allowlist for paid outbound calls. Repeatable. |
| `--max-amount-usd-per-day <MAX_AMOUNT_USD_PER_DAY>` | Replacement per-day cap in USD (advisory, not yet enforced). |
| `--max-amount-usd-per-request <MAX_AMOUNT_USD_PER_REQUEST>` | Replacement per-request cap in USD. |
| `--max-amount-usd-per-turn <MAX_AMOUNT_USD_PER_TURN>` | Replacement per-turn cap in USD (advisory, not yet enforced). |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--payment-policy-id <PAYMENT_POLICY_ID>` | Required. Payment policy's prefixed public identifier. |
| `--rail-preference <RAIL_PREFERENCE>` | Replacement rail preference, in priority order. Repeatable. |
| `--require-approval-above-usd <REQUIRE_APPROVAL_ABOVE_USD>` | Replacement approval threshold in USD (advisory, not yet enforced). |
| `--status <STATUS>` | Current lifecycle status. |

Example:

```bash
# Tighten the per-request spend cap
everruns payments policies update --payment-policy-id paypol_01h9 --max-amount-usd-per-request 2 --reason 'Cost review'
```
