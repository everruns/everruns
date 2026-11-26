---
title: Publish Applications on Kubernetes
description: Configure the Application browser host, identity provider, restricted database credentials and wildcard TLS with everruns k8s up.
sidebar:
  label: Applications
---

Applications assemble Work Chats, Rooms and shared Memory behind a stable public
address and one sign-in. Configure the browser host once per installation, then
publish individual Applications from **Build → Applications**.

## Configure the host

Start with a running platform installation (`everruns k8s up`). Its migrations
create the browser credential table before you provision the restricted login
and enable the host on a subsequent `k8s up` run.

Choose a publication domain, for example `work.example.com`. Point its wildcard
DNS record (`*.work.example.com`) at your ingress controller. Provision a wildcard
TLS certificate in the Everruns namespace, in this example `work-wildcard-tls`.
The publication domain should be separate from your console hostname.

Register an OIDC client with your identity provider. For Everruns ID, use the
issuer and client registration supplied for your deployment. Register this exact
callback URL:

```text
https://login.work.example.com/auth/callback
```

Enable authorization code flow with PKCE, the `openid profile email` scopes, and
refresh tokens (`offline_access`). Invited audiences require a verified email
claim. Organization audiences match the provider subject or a verified email to
an existing organization member.

Create a Kubernetes Secret containing `CHAT_DATABASE_URL`,
`CHAT_OIDC_CLIENT_SECRET` and `CHAT_SESSION_KEY`. The database URL must use a
dedicated PostgreSQL login with only SELECT, INSERT, UPDATE and DELETE on
`hosted_chat_credentials`, plus CONNECT on the database and USAGE on its schema.
Provision the role after the platform migrations have run; do not reuse the
platform database administrator credentials. The browser does not create tables. The session key must be 32 random bytes encoded as 64 hexadecimal
characters. For example, generate it with `openssl rand -hex 32`. Use your normal
secret manager or a protected file with `kubectl create secret --from-env-file`;
do not put secrets in shell history or Helm command arguments. Keep this key
stable across upgrades: it encrypts provider sessions stored in PostgreSQL.

Enable the host with the existing setup command:

```bash
everruns k8s up \
  --applications-domain work.example.com \
  --applications-oidc-issuer https://identity.example.com \
  --applications-oidc-client-id everruns-work \
  --applications-secret work-identity \
  --applications-tls-secret work-wildcard-tls \
  --ingress-class nginx
```

Use the actual issuer for your deployment, such as `https://id.everruns.com` when
using Everruns ID. The command retains these settings on later `k8s up` runs.
An existing installation keeps its deployed chart version; upgrade to a release
with Applications first. A checkout can be exercised with `--chart charts/everruns`
and matching locally published images.

The equivalent Helm settings are under `applications` in
[`values.yaml`](https://github.com/everruns/everruns/blob/main/charts/everruns/values.yaml). The chart runs two browser
replicas by default, uses the restricted database connection for encrypted browser
sessions, and passes matching domain/issuer/client settings to the API. The browser
Secret is mounted only into the browser deployment. The browser image is
`ghcr.io/everruns/everruns-chat-ui` at the same release tag as the server.

For a separately managed ingress, set `applications.ingress.enabled=false` and
route wildcard HTTPS traffic to the Application Service on port 3000. Preserve
`Host` and `Origin`, enable streaming responses, and allow long-lived SSE requests.
Do not expose the browser's internal HTTP Service directly to users.

## Publish a workspace

1. Open **Build → Applications**, create an Application, and choose its permanent
   stable name. A name such as `content-studio` publishes at
   `https://content-studio.work.example.com`.
2. Create or attach Chats, Rooms and Memory. A Chat can use any eligible Agent in
   the organization. Chat and Room bindings belong to one Application. Attaching
   an existing Room shares its history with the Application's audience.
3. Choose **Organization users**, **Invited people**, or **Anyone who signs in**.
   External audiences receive identities isolated to this Application; organization
   users reuse their existing accounts and connected services.
4. Save and publish. Optionally enable the launcher in the Everruns sidebar.

Shared memory is read-only unless explicitly enabled for writing. Application
agents access it at `/application/memory/<resource-name>`. Users can read it in the
Memory section and edit text files when permitted. Room participants can talk,
reply and mention the Room's agents. Configure agent members and their wake settings in the Application editor.
Automations and themes are not enabled in this release.

## Verify and operate

The Application deployment's `/health` endpoint validates required browser settings.
Open the published hostname, complete sign-in, and verify a chat, a Room post and a
Memory read. Test an excluded identity before opening an invited audience broadly.
Unpublish and confirm the next runtime request is refused; existing browser sessions
do not preserve access after a policy change.

Check the Application pod logs and OIDC callback configuration if sign-in fails.
A missing or changed session encryption key invalidates browser sessions. Do not
rotate it casually. Removing a resource stops its runtime access without deleting
its underlying history. Archiving an Application stops access and retains its name.

## Upgrade from the retired App aggregate

The migration that removes the retired App is a coordinated contract migration.
Back up PostgreSQL, drain active turns, and stop the old API and workers before
applying it. Pause autoscalers while workloads are stopped. Start only matching new
API, worker and browser images afterward. Do not perform a mixed-version rolling
upgrade across this migration; old workers and servers reference the removed schema.
A fresh `k8s up` installation has no old workloads to drain. The Application host
itself supports rolling updates once all workloads are on the new schema.
