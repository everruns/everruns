---
title: ChatGPT Plan
description: Use the ChatGPT Plan driver in a custom Everruns framework host.
appliesTo: [framework]
---

The `chatgpt` framework driver can run supported models through a personal
ChatGPT Plan allowance. It requires an OpenAI Sign in with ChatGPT grant from
the account owner. The Everruns Platform UI does not offer this provider or its
models because Everruns does not have a registered web OAuth client and callback
for this flow.

OpenAI's [open-source sign-in flow](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)
requires a `127.0.0.1` browser callback. If the host runs elsewhere, use the
framework's local login helper on the computer running the browser and transfer
the resulting credentials to the host over a protected channel. The driver
package includes a `chatgpt-login` example for this purpose. Hosts are
responsible for securing tokens and binding each connection to its owner.
The loopback helper returns a plain-text completion message by default. A host
can call `LoginAttempt::with_success_page_html` before `finish` to show its own
branded completion page after a valid callback.

The driver uses streaming, stateless Responses requests with `store=false`.
Local function and custom tools use namespaces. Web search is available when
the selected model and account support it. Text, image, and file inputs depend
on the selected model. API-only hosted capabilities, audio, and video are
unavailable. See OpenAI's [preview limitations](https://developers.openai.com/siwc/preview-limitations)
and [models and inference](https://developers.openai.com/siwc/models-and-inference).
