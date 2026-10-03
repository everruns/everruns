---
title: ChatGPT plan
description: Connect a personal ChatGPT account to self-hosted Everruns.
appliesTo: [platform]
---

Connect your personal account from **Settings → Providers → ChatGPT → Continue
with ChatGPT**. Approve plan use in ChatGPT, then return to Everruns. Available
models are discovered from your account. Choose one in the chat model picker;
the composer shows **Using your ChatGPT plan**.

This uses your ChatGPT plan and its allowance. [Manage usage in
ChatGPT](https://chatgpt.com/settings/usage). Availability depends on your account,
model, and OpenAI's open-source Sign in with ChatGPT preview.

## Enable in a self-hosted deployment

Set `FEATURE_CHATGPT_PLAN=true` and a stable `SECRETS_ENCRYPTION_KEY`. Enable
**ChatGPT plan** for the organization in its feature settings. The deployment
flag defaults to off; an organization cannot enable it when the deployment
has disabled it. Hosted deployments keep it off unless explicitly supported.

A connection belongs to its signed-in Everruns user and personal runtime
identity. Other users, other virtual identities, organization-wide defaults,
and Playground cannot use it. API providers continue to serve shared sessions.

Tokens are encrypted in the control plane. Workers receive access tokens;
refresh tokens stay in the control plane. Reconnect retains the issuing client
and account. Disconnect revokes the grant before removing credentials; a failed
revocation retains credentials so you can retry.

## Everruns runs on another computer

A browser callback goes to the computer running the browser. Use the local
login helper when that computer differs from the Everruns server:

1. Open the provider's **Everruns runs on another computer?** panel and select
   **Download login setup**. This file contains installation and registration
   identifiers, without tokens.
2. On the computer with your browser, clone the matching Everruns release and run:

   ```bash
   cargo run -p everruns-drivers --features chatgpt --example chatgpt-login -- \
     --installation everruns-chatgpt-setup.json --output everruns-chatgpt-login.json
   ```

3. Open the printed authorization URL, sign in, and allow plan use. The helper
   creates a new private credential file; it refuses to overwrite a file.
4. Upload that file in the same provider panel over HTTPS, or through a local
   SSH tunnel. Remove the credential file after a successful import. Never put
   it in Git, messages, or issue attachments.

Download a fresh setup file before reconnecting. The saved registration keeps
reconnect bound to the same account; create a separate provider for another account.

## Supported requests

The plan driver uses streaming, stateless Responses requests with `store=false`.
Local function and custom tools use namespaces. Web search is available when the
selected model and account support it. Text, image, and file inputs depend on
the selected model. API-only hosted capabilities, audio, and video are unavailable.
Use local tools or choose an API provider for unsupported capabilities.

The driver rejects incomplete streams and preserves provider usage and
eligibility errors. When plan use is declined, reconnect and allow it rather
than entering an API key. See OpenAI's [preview limitations](https://developers.openai.com/siwc/preview-limitations)
and [models and inference](https://developers.openai.com/siwc/models-and-inference).
