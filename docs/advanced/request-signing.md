---
title: Request Signing
description: How Everruns signs outbound HTTP requests with Ed25519 signatures per RFC 9421, enabling target servers to verify bot identity
sidebar:
  order: 30
---

When an AI agent fetches web content, the receiving server has no way to distinguish it from an anonymous scraper. **Request signing** solves this by attaching a cryptographic signature to every outbound HTTP request, letting target servers verify who is making the request and choose to grant or deny access based on that identity.

Everruns implements the [Web Bot Authentication Architecture](https://datatracker.ietf.org/doc/html/draft-meunier-web-bot-auth-architecture) (draft-meunier) using Ed25519 signatures over [RFC 9421 HTTP Message Signatures](https://www.rfc-editor.org/rfc/rfc9421).

## Background

### The Problem

Traditional bot identification relies on `User-Agent` strings, which are trivially spoofed. IP-based allow lists are brittle and don't scale. There is no standard way for a web bot to prove its identity to a server.

### HTTP Message Signatures (RFC 9421)

RFC 9421 defines a general mechanism for signing HTTP messages. A sender selects components of the request (method, authority, specific headers) and signs them with a private key. The signature and a description of what was signed are transmitted as structured headers:

```
Signature: sig=:base64url-encoded-signature:
Signature-Input: sig=("@authority");created=1735689600;expires=1735689900;
                  keyid="JWK-thumbprint";alg="ed25519";nonce="random";
                  tag="web-bot-auth"
```

The receiving server reconstructs the same signature base from the request, fetches the sender's public key, and verifies the signature. Replay attacks are prevented by the `created`/`expires` window and random `nonce`.

### Web Bot Authentication Architecture

The [draft-meunier-web-bot-auth-architecture](https://datatracker.ietf.org/doc/html/draft-meunier-web-bot-auth-architecture) builds on RFC 9421 specifically for bot identification:

- **Algorithm**: Ed25519 (fast, small keys, no parameter choices)
- **Covered components**: `@authority` (the target domain) at minimum
- **Key identity**: [JWK Thumbprint](https://www.rfc-editor.org/rfc/rfc7638) (SHA-256 hash of the canonical public key representation)
- **Signature tag**: `"web-bot-auth"` — distinguishes bot-auth signatures from other uses of RFC 9421
- **Discovery**: optional `Signature-Agent` header points to a FQDN where the bot's public keys can be found

### Key Discovery

The companion [draft-meunier-http-message-signatures-directory](https://datatracker.ietf.org/doc/html/draft-meunier-http-message-signatures-directory) defines how target servers find a bot's public keys:

```
GET https://<signature-agent-fqdn>/.well-known/http-message-signatures-directory
```

This returns a [JSON Web Key Set (JWKS)](https://www.rfc-editor.org/rfc/rfc7517#section-5) containing the bot's Ed25519 public keys. The target server uses the `kid` field to match the key against the `keyid` in the incoming signature.

```
┌──────────┐         ┌──────────────┐         ┌─────────────┐
│  Agent   │──GET──►│ Target Server │         │  Bot's FQDN │
│ (signed) │        │              │──GET──►│ /.well-known │
│          │        │  1. extract  │        │ /http-msg-   │
│          │        │     keyid    │◄──JWKS─│  sig-dir     │
│          │        │  2. verify   │         │              │
│          │        │     sig      │         │              │
└──────────┘        └──────────────┘         └─────────────┘
```

## How It Works in Everruns

Request signing is implemented as a server-wide feature. When enabled, **every outbound HTTP request** made by the `web_fetch` tool is signed.

### Signing (outbound)

The signing pipeline is handled by [fetchkit](https://github.com/everruns/fetchkit), the library powering the `web_fetch` capability:

1. Agent calls `web_fetch` with a URL
2. fetchkit builds the HTTP request
3. If bot-auth is configured, fetchkit signs the request:
   - Covers `@authority` and optionally `signature-agent`
   - Generates a random nonce
   - Sets `created` and `expires` timestamps
   - Signs with Ed25519, attaches `Signature` and `Signature-Input` headers
4. If signing fails (clock error, etc.), the request proceeds unsigned with a warning logged — signing never blocks requests
5. The request is sent to the target server

### Key directory (inbound)

Everruns serves the public key at `/.well-known/http-message-signatures-directory`. This endpoint:

- Is **public** (no authentication required)
- Returns a JWKS containing the server's Ed25519 public key
- Derives the key at startup from the same seed used for signing

Example response:

```json
{
  "keys": [
    {
      "kty": "OKP",
      "crv": "Ed25519",
      "x": "base64url-encoded-public-key",
      "kid": "JWK-thumbprint-matching-keyid-in-signatures"
    }
  ]
}
```

### What target servers see

A signed request arrives with three additional headers:

| Header | Purpose |
|--------|---------|
| `Signature` | The Ed25519 signature over the covered components |
| `Signature-Input` | Describes what was signed: components, timestamps, key ID, algorithm, nonce |
| `Signature-Agent` | FQDN where the bot's public keys can be discovered (optional) |

Target servers that support web-bot-auth can:
1. Extract the `keyid` from `Signature-Input`
2. Fetch the public key from the `Signature-Agent` FQDN's well-known endpoint
3. Verify the signature
4. Apply access policies based on the verified identity

Servers that don't support it simply ignore the extra headers.

## Verifying Signatures (Server Side)

If you operate a server that receives requests from Everruns agents, here's how to verify them.

### Verification steps

1. **Check the tag** — parse `Signature-Input` and confirm `tag="web-bot-auth"`. Ignore signatures with other tags.
2. **Check timestamps** — reject if `created` is in the future or `expires` is in the past. A 5-minute clock skew tolerance is reasonable.
3. **Fetch the public key** — extract the `Signature-Agent` FQDN and fetch `https://<fqdn>/.well-known/http-message-signatures-directory`. Find the key matching the `keyid` from `Signature-Input`. Cache the JWKS (keys rotate infrequently).
4. **Reconstruct the signature base** — build the canonical representation per [RFC 9421 Section 2.5](https://www.rfc-editor.org/rfc/rfc9421#section-2.5) using the covered components listed in `Signature-Input`.
5. **Verify** — use Ed25519 to verify the signature against the reconstructed base and the fetched public key.

### Python example

```python
import base64
import hashlib
import time
import httpx
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

def verify_bot_auth(request) -> bool:
    """Verify a web-bot-auth signature on an incoming request."""

    # 1. Parse Signature-Input header
    sig_input = request.headers.get("signature-input", "")
    if 'tag="web-bot-auth"' not in sig_input:
        return False  # not a bot-auth signature

    # Extract parameters from sig_input
    # sig=("@authority" "signature-agent");created=...;expires=...;keyid="...";...
    params = parse_signature_params(sig_input)

    # 2. Check timestamps
    now = int(time.time())
    if params["created"] > now + 300 or params["expires"] < now:
        return False  # expired or future-dated

    # 3. Fetch public key from Signature-Agent FQDN
    agent_fqdn = request.headers.get("signature-agent", "")
    jwks_url = f"https://{agent_fqdn}/.well-known/http-message-signatures-directory"
    jwks = httpx.get(jwks_url).json()
    key_data = next(k for k in jwks["keys"] if k.get("kid") == params["keyid"])
    public_key_bytes = base64.urlsafe_b64decode(key_data["x"] + "==")
    public_key = Ed25519PublicKey.from_public_bytes(public_key_bytes)

    # 4. Reconstruct signature base (RFC 9421 Section 2.5)
    #    Covered components are listed in parentheses in Signature-Input
    sig_base = build_signature_base(request, params)

    # 5. Verify
    signature = base64.b64decode(
        request.headers["signature"].split(":")[1]  # sig=:base64:
    )
    try:
        public_key.verify(signature, sig_base.encode())
        return True
    except Exception:
        return False
```

### Node.js example

```javascript
import { createPublicKey, verify } from "node:crypto";

async function verifyBotAuth(request) {
  const sigInput = request.headers["signature-input"] || "";
  if (!sigInput.includes('tag="web-bot-auth"')) return false;

  const params = parseSignatureParams(sigInput);

  // Check timestamps (5-minute tolerance)
  const now = Math.floor(Date.now() / 1000);
  if (params.created > now + 300 || params.expires < now) return false;

  // Fetch public key
  const fqdn = request.headers["signature-agent"];
  const res = await fetch(
    `https://${fqdn}/.well-known/http-message-signatures-directory`
  );
  const jwks = await res.json();
  const jwk = jwks.keys.find((k) => k.kid === params.keyid);

  const key = createPublicKey({ key: jwk, format: "jwk" });

  // Reconstruct signature base and verify
  const sigBase = buildSignatureBase(request, params);
  const signature = Buffer.from(
    request.headers["signature"].split(":")[1],
    "base64"
  );

  return verify(null, Buffer.from(sigBase), key, signature);
}
```

> **Note:** The `parseSignatureParams` and `buildSignatureBase` helpers follow the structured fields parsing rules from [RFC 8941](https://www.rfc-editor.org/rfc/rfc8941) and the signature base construction from [RFC 9421 Section 2.5](https://www.rfc-editor.org/rfc/rfc9421#section-2.5). Libraries like [httpbis-message-signatures](https://pypi.org/project/httpbis-message-signatures/) (Python) and [@httpbis/message-signatures](https://www.npmjs.com/package/@httpbis/message-signatures) (Node.js) handle both.

## Configuration

Request signing is configured via environment variables. Set them before starting the server.

### Environment variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BOT_AUTH_SIGNING_KEY_SEED` | yes | — | Base64url-encoded 32-byte Ed25519 seed |
| `BOT_AUTH_AGENT_FQDN` | no | — | FQDN for the `Signature-Agent` header |
| `BOT_AUTH_VALIDITY_SECS` | no | `300` | Signature validity window in seconds |

When `BOT_AUTH_SIGNING_KEY_SEED` is not set, signing is disabled and no crypto code runs at request time.

### Generate a signing key

```bash
python3 -c "import os, base64; print(base64.urlsafe_b64encode(os.urandom(32)).rstrip(b'=').decode())"
```

### Enable signing

```bash
export BOT_AUTH_SIGNING_KEY_SEED="your-base64url-seed-here"
export BOT_AUTH_AGENT_FQDN="bot.yourcompany.com"
```

Then start the server. All `web_fetch` requests will be signed, and the public key will be available at `https://bot.yourcompany.com/.well-known/http-message-signatures-directory`.

### Verify it's working

```bash
# Check the key directory endpoint
curl -s http://localhost:9301/.well-known/http-message-signatures-directory | jq .
```

## Standards

| Standard | Role |
|----------|------|
| [RFC 9421 — HTTP Message Signatures](https://www.rfc-editor.org/rfc/rfc9421) | Core signing mechanism — how to sign and verify HTTP requests |
| [RFC 8941 — Structured Field Values](https://www.rfc-editor.org/rfc/rfc8941) | Encoding format for `Signature` and `Signature-Input` headers |
| [RFC 7638 — JWK Thumbprint](https://www.rfc-editor.org/rfc/rfc7638) | How the `keyid` is computed from the public key |
| [RFC 7517 — JSON Web Key (JWK)](https://www.rfc-editor.org/rfc/rfc7517) | Format of the keys in the JWKS response |
| [RFC 8037 — Ed25519 in JOSE](https://www.rfc-editor.org/rfc/rfc8037) | Ed25519 key representation in JWK format |
| [draft-meunier-web-bot-auth-architecture](https://datatracker.ietf.org/doc/html/draft-meunier-web-bot-auth-architecture) | Bot-specific profile of RFC 9421 (algorithm, tag, covered components) |
| [draft-meunier-http-message-signatures-directory](https://datatracker.ietf.org/doc/html/draft-meunier-http-message-signatures-directory) | Well-known endpoint for public key discovery |

## See Also

- [fetchkit](https://github.com/everruns/fetchkit) — The library implementing the signing client
