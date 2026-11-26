---
title: "Publish Applications with Chats, Rooms and Memory"
description: "Build a named workspace with multiple Agents, collaborative Rooms and shared Memory, then publish it with one audience and its own sign-in."
date: 2026-11-25
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/applications/, /sre/runbooks/applications/]
---

[Applications](/features/applications/) bring multiple Work Chats, Rooms and shared
Memory into one published workspace. In **Build → Applications**, create resources
or attach existing ones, choose their navigation labels, configure the audience,
and publish at a permanent name. An optional launcher appears in the Everruns sidebar.

![A published Content Studio Application with two Chats, an editorial Room and shared Memory](./overview@2x.png)

External audiences receive an Application-specific identity. Organization audiences
reuse existing member accounts. Chats keep private conversations; Rooms and Memory
share only what the author explicitly includes. Memory write access is opt-in.

![Editing the Application's shared brand handbook](./memory@2x.png)

Self-hosted installations can enable the browser host with the new
`everruns k8s up --applications-domain` options. See the
[Kubernetes setup guide](/sre/runbooks/applications/) for identity, DNS, TLS and
restricted database credentials. Automations remain reserved for the future runtime.
