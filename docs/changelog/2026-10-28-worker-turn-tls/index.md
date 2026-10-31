---
title: Worker turn requests now honor the configured TLS settings
description: Worker turn RPCs and blob fetches now apply the same optional TLS and client certificate settings as task requests, with bounded handshake failures.
date: 2026-10-28
kind: fix
significance: notable
products: [platform]
release: "0.50.0"
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
---

Workers now apply `WORKER_GRPC_TLS_CA_CERT`, `WORKER_GRPC_TLS_DOMAIN` and optional
client certificate settings to turn requests and their blob fetches. Previously,
only the durable/task client applied these settings. Configured TLS verifies the
server name, refuses certificate errors without a plaintext fallback, and limits
turn connection and handshake time using `WORKER_GRPC_REQUEST_TIMEOUT_SECS`.

Before upgrading a TLS-configured worker, verify that its CA, expected server
name and optional client identity are valid for the control plane. Workers with
an unset or empty CA setting retain plaintext transport. See
[Environment variables](/sre/environment-variables/).

TLS protects the transport connection; tenant and assignment authorization still
depend on application checks and worker isolation.
