---
title: "SDK 0.2.1 release"
description: "The Rust SDK can use native TLS for smaller binaries and moves to reqwest 0.13, with dependency and security updates across all three SDKs."
date: 2026-08-19
kind: release
significance: minor
products: [sdk]
release: "0.2.1"
---

## Highlights

- Rust SDK footprint trimmed for consumer binaries: leaner tokio/futures/reqwest feature sets, and TLS is now a Cargo feature (default `rustls-tls`, or `native-tls` for a much smaller binary).
- Rust SDK upgraded to reqwest 0.13.
- Dependency and security updates across all SDKs.
- Documentation polish: Everruns logo in the README, plainer punctuation in prose.

## In the changelog

- [Everruns Rust SDK can use native TLS for smaller binaries](/changelog/2026-08-19-sdk-rust-native-tls-feature/)

See the [GitHub release](https://github.com/everruns/sdk/releases/tag/v0.2.1) and the [full release notes](https://github.com/everruns/sdk/blob/main/CHANGELOG.md#021---2026-08-19) for every pull request in this version.
