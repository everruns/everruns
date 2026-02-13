# Messaging Channels

## Abstract

Messaging channels connect Everruns sessions to external chat platforms (WhatsApp, Discord, Slack, Signal, etc.). A channel adapter translates between platform-native protocols and the Everruns session/message API, enabling agents to converse on real messaging surfaces without changes to the core agent loop.

## Analysis: OpenClaw Channel Architecture

OpenClaw is a TypeScript/Node.js personal AI assistant that connects to 20+ messaging platforms. Key architectural patterns extracted below.

### Gateway + Plugin Model

OpenClaw runs a long-lived **Gateway** daemon that holds persistent connections to each platform. Channels are **plugins** that register via a standard `ChannelPlugin<Account>` interface:

```
Gateway (daemon)
├── Channel Plugin: WhatsApp (Baileys WebSocket)
├── Channel Plugin: Discord (discord.js gateway)
├── Channel Plugin: Slack (Bolt socket/HTTP)
├── Channel Plugin: Signal (signal-cli JSON-RPC + SSE)
└── ...
```

Each plugin declares:
- **Metadata** — id, label, docs path, icon
- **Account lifecycle** — `startAccount()`, `stopAccount()`, `loginWithQr*()`, `logoutAccount()`, `checkReady()`
- **Message sending** — `sendText()`, `sendMedia()`, `sendPoll()`
- **Access control** — DM policy (pairing/allowlist/open/disabled), group policy, mention gating
- **Routing** — session key derivation from channel + account + peer/group IDs
- **Actions** — platform-native operations (react, pin, delete, edit)

### Message Flow

```
Platform (WhatsApp/Discord/...)
    │
    ▼ (inbound: platform SDK callback)
Gateway.monitorChannel()
    │
    ▼
Normalize to envelope {sender, chatType, text, media, replyContext}
    │
    ▼
resolveAgentRoute(channel, account, peer, guild, roles)
    │  → picks agent via binding hierarchy
    │  → derives sessionKey: "agent:{id}:{channel}:{account}:{peer}"
    │
    ▼
Agent session (create or resume)
    │
    ▼ (outbound: agent response)
Channel.sendText() / sendMedia()
    │
    ▼
Platform (WhatsApp/Discord/...)
```

### Channel-Specific Details

**WhatsApp** — Baileys (reverse-engineered WhatsApp Web). Gateway owns the linked session. QR pairing for auth. Credentials stored on disk. Text chunked to 4000 chars. Media auto-optimized (5MB limit). E.164 phone number identity. Group JID routing.

**Discord** — discord.js gateway with Message Content + Server Members intents. Bot token auth. Role-based agent routing. Thread sessions inherit from parent channel. Slash commands run in isolated sessions. PluralKit proxy support.

**Slack** — Bolt SDK with Socket Mode (WebSocket) or HTTP Events API. Dual tokens: bot (`xoxb-`) + app (`xapp-`). Optional user token for reads. Channel mention-gated by default. Thread-level session scoping.

**Signal** — signal-cli daemon via HTTP JSON-RPC + SSE. Separate phone number required. Time-limited pairing codes (1h expiry). Base64 media attachments (8MB default). E.164 identity.

### Key Patterns

1. **Per-account isolation** — each account gets its own abort controller, credential store, status tracker
2. **Deterministic session keys** — `agent:{agentId}:{channel}:{accountId}:{peerId}` ensures message continuity
3. **Normalized envelope** — all platforms funnel into the same inbound format before routing
4. **Hierarchical access control** — DM policy → group policy → sender allowlist → mention gating
5. **Plugin config schema** — each plugin declares a JSON schema for its settings; validated on load
6. **Lazy loading** — channel implementations loaded only when enabled (avoids pulling in heavy SDKs)

## Proposed Design for Everruns

### Guiding Principles

- Channels are **adapters**, not core. The agent loop, events, sessions remain unchanged.
- Channel code lives in separate crates; the server has zero compile-time dependency on platform SDKs.
- Inbound messages go through the existing `POST /v1/sessions/{session_id}/messages` path.
- Outbound delivery reacts to events via the existing `EventListener` trait.

### Architecture

```
                     ┌──────────────────────────────────┐
                     │          Everruns Server          │
                     │  (unchanged: API, services, DB)  │
                     └────────────┬─────────────────────┘
                                  │ EventListener (outbound)
                                  │ REST API (inbound)
                                  │
                     ┌────────────▼─────────────────────┐
                     │       Channel Gateway Crate       │
                     │  - ChannelAdapter trait            │
                     │  - ChannelRouter (session lookup)  │
                     │  - InboundNormalizer               │
                     │  - OutboundDispatcher              │
                     │  - ChannelRegistry                 │
                     └────────────┬─────────────────────┘
                                  │
              ┌───────────────────┼───────────────────────┐
              │                   │                       │
   ┌──────────▼──────┐ ┌─────────▼────────┐ ┌────────────▼──────┐
   │ WhatsApp Adapter │ │ Discord Adapter  │ │  Slack Adapter    │
   │ (Baileys/HTTP)   │ │ (Serenity/HTTP)  │ │  (Bolt/HTTP)     │
   └─────────────────┘ └──────────────────┘ └───────────────────┘
```

### Core Trait: `ChannelAdapter`

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync + 'static {
    /// Unique channel identifier (e.g., "whatsapp", "discord", "slack")
    fn channel_id(&self) -> &str;

    /// Human-readable display name
    fn display_name(&self) -> &str;

    /// Start listening for inbound messages. Implementations hold
    /// persistent connections (WebSocket, long-poll, webhook server).
    /// The `sender` is used to push normalized inbound messages
    /// to the gateway for routing into Everruns sessions.
    async fn start(
        &self,
        config: ChannelConfig,
        sender: InboundSender,
        cancel: CancellationToken,
    ) -> Result<()>;

    /// Send a message to the platform.
    async fn send_message(
        &self,
        target: &OutboundTarget,
        content: &OutboundContent,
    ) -> Result<SendReceipt>;

    /// Check if the adapter is connected and ready.
    async fn health(&self) -> ChannelHealth;
}
```

### Inbound Flow

```
Platform SDK callback
    │
    ▼
ChannelAdapter normalizes → InboundMessage {
    channel_id: "whatsapp",
    account_id: "default",
    sender: SenderIdentity { id, name, phone?, ... },
    chat_type: ChatType::Direct | Group { id, name },
    content: Vec<ContentPart>,  // reuse core type
    reply_to: Option<ExternalMessageId>,
    raw_platform_id: String,
}
    │
    ▼
InboundSender::send(inbound)
    │
    ▼
ChannelRouter:
    1. Resolve or create session
       - key: "{org}:{channel}:{account}:{chat_type}:{peer_or_group}"
       - lookup channels_sessions table
       - if not found → POST /v1/sessions (create with agent binding)
    2. Map sender → Everruns user or metadata
    3. POST /v1/sessions/{session_id}/messages with content
    │
    ▼
Normal Everruns processing (turn execution, events, SSE)
```

### Outbound Flow

```
EventListener receives output.message.completed event
    │
    ▼
OutboundDispatcher:
    1. Lookup channels_sessions for this session_id
    2. If session is channel-bound → resolve adapter + target
    3. Convert ContentPart[] → OutboundContent (text chunks, media)
    4. Call adapter.send_message(target, content)
    5. Store delivery receipt as event metadata
```

### Data Model

#### `channel_accounts` Table

Per-organization channel account registration.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID PK | |
| `organization_id` | UUID FK | Parent org |
| `channel_id` | TEXT | `"whatsapp"`, `"discord"`, `"slack"`, `"signal"` |
| `account_id` | TEXT | Platform-specific (phone number, bot ID, workspace) |
| `label` | TEXT | Human-readable label |
| `agent_id` | UUID FK? | Default agent for this account |
| `config` | JSONB | Channel-specific config (encrypted at rest) |
| `status` | TEXT | `active`, `paused`, `error` |
| `created_at` / `updated_at` | TIMESTAMP | |

#### `channel_sessions` Table

Maps external chat identities to Everruns sessions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID PK | |
| `channel_account_id` | UUID FK | Which channel account |
| `external_chat_id` | TEXT | Platform peer/group/channel ID |
| `chat_type` | TEXT | `direct`, `group`, `channel` |
| `session_id` | UUID FK | Everruns session |
| `metadata` | JSONB | Sender info, group name, etc. |
| `created_at` / `updated_at` | TIMESTAMP | |

**Unique constraint:** `(channel_account_id, external_chat_id)` — one Everruns session per external chat.

#### `channel_messages` Table

Maps platform message IDs to Everruns message IDs for reply threading.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID PK | |
| `channel_session_id` | UUID FK | |
| `external_message_id` | TEXT | Platform message ID |
| `everruns_message_id` | UUID | Everruns message ID |
| `direction` | TEXT | `inbound` / `outbound` |
| `created_at` | TIMESTAMP | |

### Access Control

Follows OpenClaw's layered model:

1. **DM policy** — `pairing` (time-limited code), `allowlist` (explicit phone/user IDs), `open`, `disabled`
2. **Group policy** — `open`, `allowlist` (explicit group IDs), `disabled`
3. **Mention gating** — for group chats, only respond when explicitly mentioned (configurable per channel)
4. **Sender allowlist** — E.164 phone numbers (WhatsApp/Signal) or platform user IDs (Discord/Slack)

### Channels to Implement

| Channel | Protocol | Auth | Priority |
|---------|----------|------|----------|
| **WhatsApp** | Baileys (WhatsApp Web) or WhatsApp Cloud API | QR link or API token | High |
| **Discord** | discord.js or Serenity (Rust) | Bot token | High |
| **Slack** | Bolt SDK (Socket Mode or HTTP) | Bot + App tokens | High |
| **Signal** | signal-cli (JSON-RPC + SSE) | Phone number + pairing | Medium |
| **Google Chat** | Chat API (HTTP) | Service account | Medium |
| **iMessage** | BlueBubbles API | BlueBubbles server | Low |
| **Microsoft Teams** | Bot Framework | Azure AD app | Low |
| **Matrix** | Matrix client-server API | Access token | Low |
| **IRC** | IRC protocol | Nick + server | Low |

**Excluded:** Telegram (per requirements).

### Implementation Strategy

Adapters can be implemented in two ways:

1. **Rust-native** — for platforms with mature Rust crates (Discord via serenity, Matrix via matrix-sdk, IRC via irc crate). Compiled into a `channels-gateway` binary or loaded as a feature-gated module.

2. **Sidecar process** — for platforms requiring Node.js SDKs (WhatsApp/Baileys, Slack/Bolt). A lightweight TypeScript sidecar runs the platform SDK and communicates with the Rust gateway via HTTP/gRPC. Similar to how OpenClaw's signal-cli integration works via JSON-RPC.

### Crate Structure

```
crates/
├── channels/           # Channel gateway core (trait, router, dispatcher)
├── channels-whatsapp/  # WhatsApp adapter (sidecar + Rust glue)
├── channels-discord/   # Discord adapter (serenity)
├── channels-slack/     # Slack adapter (sidecar)
├── channels-signal/    # Signal adapter (signal-cli sidecar)
└── ...
```

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/channels` | List available channel types |
| POST | `/v1/channel-accounts` | Register a channel account |
| GET | `/v1/channel-accounts` | List channel accounts |
| GET | `/v1/channel-accounts/{id}` | Get channel account |
| PATCH | `/v1/channel-accounts/{id}` | Update channel account config |
| DELETE | `/v1/channel-accounts/{id}` | Remove channel account |
| POST | `/v1/channel-accounts/{id}/login` | Initiate platform login (QR, OAuth, etc.) |
| GET | `/v1/channel-accounts/{id}/status` | Connection status + health |
| GET | `/v1/channel-accounts/{id}/sessions` | List sessions for this channel account |

### Differences from OpenClaw

| Aspect | OpenClaw | Everruns |
|--------|----------|---------|
| Language | TypeScript/Node.js | Rust (with TS sidecar for some SDKs) |
| Deployment | Single gateway daemon | Separate channel gateway process + server |
| Session model | String-based session keys | UUID-based sessions with DB FK |
| Message storage | Direct store | Event-sourced (messages are events) |
| Agent routing | Config-file bindings | DB-stored channel_accounts with agent_id FK |
| Access control | Config-driven | DB-stored per channel_account |
| Plugin model | Runtime JS plugin registry | Compile-time Rust trait impls + sidecar |

### Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Baileys is unofficial, may break | Support WhatsApp Cloud API as alternative; abstract behind trait |
| Node.js sidecar adds operational complexity | Containerize sidecar with channel gateway; health checks |
| Platform rate limits | Per-channel rate limiter in OutboundDispatcher |
| Message ordering across platforms | Use event sequence numbers; don't guarantee cross-platform ordering |
| Credential security | Store in `channel_accounts.config` JSONB with envelope encryption (existing pattern) |
| Large media files | Proxy through session filesystem; respect per-platform size limits |

### Open Questions

1. Should channels run in-process (feature-gated crate dependencies) or as separate binaries?
2. WhatsApp: Baileys (unofficial, full-featured) vs Cloud API (official, limited, requires Meta business account)?
3. Should the channel gateway be a separate deployable or embedded in the server?
4. How to handle multi-turn context when platform messages arrive faster than agent processing?
