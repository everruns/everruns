---
title: "serve keeps daemon data in S3 and can require API keys"
description: "serve start --store s3:// keeps a daemon's data in an object store, agent routes can require API keys or OIDC, and serve resumes cut-off sessions at boot."
date: 2026-10-10
kind: feature
significance: notable
products: [framework]
surfaces: [code, cli, operations]
links:
  docs: [/framework/serve/#keep-the-data-in-a-bucket, /framework/serve/#authentication]
  prs: [everruns/everruns#4515, everruns/everruns#4518, everruns/everruns#4485]
---

A serve daemon can now keep its data directory in an S3 bucket or any
S3-compatible store with conditional writes:

```bash
./revenue-analyst start --store s3://bucket/prefix
```

A daemon started elsewhere with the same store waits out a killed holder's
lease, restores the data and carries on.

serve apps can also require a credential on agent routes: static API keys from
code, `serve.toml` or `SERVE_API_KEYS`, OIDC, or OAuth 2.0 introspection. The
agent card stays public. At boot, serve resumes sessions active in the last
seven days and re-runs an unfinished tool call only when that is safe.

See [Keep the data in a bucket](/framework/serve/#keep-the-data-in-a-bucket),
[Authentication](/framework/serve/#authentication) and [#4515](https://github.com/everruns/everruns/pull/4515), [#4518](https://github.com/everruns/everruns/pull/4518), [#4485](https://github.com/everruns/everruns/pull/4485).
