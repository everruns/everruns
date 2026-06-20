---
title: AWS Bedrock
description: Run Everruns agents on models hosted in Amazon Bedrock via the ConverseStream API, with AWS credential and region resolution.
sidebar:
  label: AWS Bedrock
---

<svg role="img" aria-label="AWS Bedrock logo" width="56" height="56" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>

Everruns runs agents on models hosted in
[Amazon Bedrock](https://aws.amazon.com/bedrock/) through the Bedrock Runtime
`ConverseStream` API, mapping its provider-neutral messages, tools, and reasoning
onto the Bedrock wire format.

## What you get

- **Bedrock Runtime `ConverseStream`** streaming.
- **Tool calls and reasoning** mapped to provider-neutral Everruns types.
- **AWS credential and region** resolution from explicit fields.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **AWS Bedrock**.
3. Enter your AWS credentials as separate fields:
   - **Access key ID**
   - **Secret access key**
   - **Region** (e.g. `us-east-1`)
   - **Session token** (optional, for temporary credentials)
4. Save. Make sure the models you want are enabled in your Bedrock account.

Unlike most providers, Bedrock uses a multi-field credential rather than a single
API key, so each field has its own input.

## Models

Enable the model access you need in the
[Amazon Bedrock console](https://console.aws.amazon.com/bedrock/) first. Everruns
resolves Bedrock model ids to its built-in model profiles for capability and cost
metadata.

## Links

- [Amazon Bedrock](https://aws.amazon.com/bedrock/)
- [Amazon Bedrock console](https://console.aws.amazon.com/bedrock/)
- [`everruns-bedrock` on crates.io](https://crates.io/crates/everruns-bedrock)
- [Migrate between providers](/how-to/migrate-providers/)
