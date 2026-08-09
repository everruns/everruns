---
type: Specification
title: "Framework Provider Extension Boundary"
description: "Why credential-free model identity is separate from open provider execution."
tags:
  - everruns
  - framework
  - providers
---

# Framework Provider Extension Boundary

## Intent

Model selection is application configuration; provider execution is an open
extension. The Framework therefore keeps model identity credential-free and
pairs it with a provider assembly that supplies protocol behavior, endpoint
configuration, and authentication at the trusted host boundary.

This separation lets applications name, compare, store, and route models
without carrying secrets. It also lets a new provider integrate without adding
a closed model variant or teaching application code provider-specific
branches.

## Contract

- Model identity must be safe to log and move through application values.
- Provider configuration owns credentials and must redact them from diagnostic
  output.
- Built-in provider conveniences and custom providers converge on the same
  resolution and execution path.
- Provider-specific protocol differences remain behind the driver boundary.
- Unknown, duplicate, or mismatched provider identities fail explicitly before
  they can select an unintended implementation.

The exact driver trait, provider constructors, and stream events live in the
public API reference. Framework usage lives in the public [Models and
Providers](../../docs/framework/models-and-providers.md) and [Custom
Providers](../../docs/framework/custom-providers.md) guides. Application versus
host ownership remains canonical in [Application API
Boundaries](application-api.md).
