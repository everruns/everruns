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
pairs a plain provider-visible model id with a provider assembly that supplies
protocol behavior, endpoint configuration, and authentication at the trusted
host boundary. An ordinary Framework agent accepts one provider; the facade
constructs the execution-facing `ModelSpec` internally when the agent builds.

This separation lets applications name, compare, store, and route models
without carrying secrets. It also lets a new provider integrate without adding
a closed model variant or teaching application code provider-specific
branches.

## Contract

- Model identity must be safe to log and move through application values.
- Provider configuration owns credentials and must redact them from diagnostic
  output.
- Application code selects a live model with `.provider(provider).model(id)`;
  it does not construct a `ModelSpec` or bundle provider configuration into a
  model value.
- An agent accepts exactly one provider until the Framework has a concrete
  application-level multi-provider routing contract. Missing or multiple
  providers fail during agent construction.
- Deterministic simulation may bundle its private offline provider because it
  has no endpoint or credential configuration.
- Built-in provider conveniences and custom providers converge on the same
  resolution and execution path.
- Provider-specific protocol differences remain behind the driver boundary.

The exact driver trait, provider constructors, and stream events live in the
public API reference. Framework usage lives in the public [Models and
Providers](../../docs/framework/models-and-providers.md) and [Custom
Providers](../../docs/framework/custom-providers.md) guides. Application versus
host ownership remains canonical in [Application API
Boundaries](application-api.md).
