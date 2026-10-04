---
type: Specification
title: "Session Secret Reads"
description: "Why reading a session secret returns plaintext, and what that does and does not protect."
tags:
  - everruns
  - security
---
# Session Secret Reads

## Abstract

A session secret is encrypted at rest (see [Encryption Specification](encryption.md)) and decrypted by the control plane before it reaches the caller. Nothing between the control plane and the consumer is encrypted, and nothing is meant to be: the consumers need the value itself. This document records which consumers exist, what the current design protects, what it does not, and which lever would change that — so the decision is not re-derived from the code each time someone asks whether secrets "should be encrypted" on the way out.

## The two consumers

Reading a secret value serves two unrelated purposes, and they want different guarantees.

| Consumer | How it reads | Where the value ends up |
|---|---|---|
| The agent | the `session_storage` capability's `get` operation | the tool result, and therefore the model's context |
| Infrastructure | MCP secret bindings and `McpServerInfo.api_key` | injected into an MCP server's environment or headers |

The agent consumer is the one that governs. Whatever the transport does, the value is handed to a model as text, and from there it is in the prompt, the transcript, and any trace that captures tool results. The infrastructure consumer never shows the value to a model; it only needs it at the point of injection.

## What the current design protects

- **At rest.** The stored value is ciphertext; only the control plane holds `SECRETS_ENCRYPTION_KEY`. The worker holds no key material and links no cryptography.
- **Against tenants.** Every read verifies session ownership against the caller's org before touching the row, and reserved internal names answer as absent rather than confirming they exist.
- **Against the REST surface.** The HTTP API offers list-names, batch-set and delete. It has never offered a value read, and `get_session_secret` deliberately does not add one.

## What it does not protect

- **The value in the model's context.** No transport change touches this. Encrypting the control-plane-to-worker hop would still end with the worker handing plaintext to the model, because that is what the capability is for.
- **Plaintext in worker-side logs and traces.** The worker necessarily holds the value to use it.
- **A worker that claims an org.** `Caller::internal` carries the Owner role and bypasses policy evaluation (TM-AUTHZ-002), so a read on the command transport is gated by reachability rather than by policy.

## Why not encrypt the hop

Encryption helps only when the decryption point is *later* than the exposure point. Both consumers decrypt at the worker, so an envelope-encrypted response narrows the window without closing it: the value is still plaintext in the worker, still plaintext in the model's context, and now also costs a new keypair, its distribution, and its rotation for a principal that holds no key material today. The cost buys a smaller window, not a smaller blast radius.

The lever that would change the threat model is **late binding**: the command returns an opaque handle, the worker passes the handle to the injection site, and the control plane substitutes the real value — at sandbox provisioning, or through a server-side egress proxy. That removes worker-side plaintext for the infrastructure consumer entirely. It does nothing for the agent consumer, which is asking to see the value, and whose exposure is a product question — whether an agent should read a raw secret at all, or only ask the platform to apply one — rather than a plumbing question.

## Why the read is a command with no route and no scripting exposure

`get_session_secret` exists so the worker reads secrets over the same command transport as everything else, which puts ownership verification, the reserved-name rule and decryption in one place instead of re-implementing them per transport. It replaced a bespoke `SessionStorageGetSecret` RPC.

Registering a command has a consequence worth stating plainly, because it is not obvious from the command definition: **the MCP catalog exposes every command in the inventory as a scripting tool.** The only other exclusion is a path prefix for durable control-plane internals. So a command added for the worker is, by default, also an MCP tool. For a command that returns a decrypted credential through a surface where REST deliberately offers no value read, that default is wrong, and `exposed_to_scripting` excludes it by name with a test that fails if the exclusion is removed.

The net reachability is therefore unchanged from the RPC it replaced: the worker, and the agent capability that already had it. The command's policy is `SESSION_MANAGE` rather than the `SESSION_VIEW` its metadata siblings use, since it returns the credential rather than facts about it — defence in depth for whatever caller arrives next, not the control that matters today. The control that matters is reachability.

## Deployment ordering

Removing the RPC means a worker older than the server calls a method the server no longer serves and gets `Unimplemented`. Deploy the server and worker together, or accept that secret reads fail for one roll. The failure is loud rather than silent.

## Related

- [Encryption Specification](encryption.md) — envelope encryption and key rotation.
- [Secret-leak Guardrails](secret-leak-guardrails.md) — redaction of known values in model-facing output.
- [Threat Model](threat-model.md) — TM-AUTHZ-002 (internal callers bypass policy), TM-AUTHZ-023 (credential metadata).
