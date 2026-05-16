# TC004 FCP channel — create, configure, and verify endpoint

## Description

Verifies that an FCP channel can be created from the Apps UI, that the form
exposes every FCP-specific option (token, handshake override, expiration,
per-IP rate limit, response timeout), and that the saved channel's setup
guidance shows the public endpoint and the right access badge.

Also verifies the GET handshake renders the configured Markdown.

## Preconditions

- `AUTH_MODE=none`
- `FEATURE_APPS_DETAIL_V2=true`
- `just start-dev --no-watch` is running
- A draft or published App exists with an agent and harness assigned

## Test Data

| Field                       | Value                                                    |
| --------------------------- | -------------------------------------------------------- |
| Route                       | `/apps/{appId}/channels/new`                             |
| Channel type                | FCP                                                      |
| Anonymous toggle            | On                                                       |
| Bearer token                | _(use the Regenerate button)_                            |
| Custom handshake (Markdown) | `# Try me\n\nPOST plain text to chat with this agent.`   |
| Session expiration (hours)  | `6`                                                      |
| Rate limit / minute         | `30`                                                     |
| Response timeout (seconds)  | `90`                                                     |

## Steps

1. Navigate to the App detail page → Add channel.
2. Verify the channel type picker includes a "FCP (Free Communication
   Protocol)" card alongside Schedule, Webhook, AG-UI, and Slack.
3. Select FCP. The form should reveal:
   - Anonymous-access toggle (default on).
   - Bearer token input with Regenerate button.
   - Custom handshake (Markdown) textarea.
   - Session expiration (hours) input.
   - Rate limit / minute input.
   - Response timeout (seconds) input (default `120`).
4. Click the Regenerate button next to Bearer token. The token field fills with
   a fresh random token.
5. Paste the custom handshake from Test Data into the textarea.
6. Set rate limit to `30` and response timeout to `90`. Leave anonymous toggle
   on; leave expiration at `6`.
7. Click Save channel.
8. From the channel list, open the new FCP channel. Verify the setup guidance
   panel shows:
   - Access badge `Token Protected` (the token field is configured).
   - The public endpoint URL (`/api/v1/apps/{appId}/fcp`) with a Copy button.
   - The "Custom Markdown configured." note for the handshake.
   - Session expiration `6 hours` and rate limit `30 requests per minute, per
     client IP`.
   - Response timeout `90 seconds`.
9. From a terminal, run:
   ```bash
   curl -s -i http://localhost:9300/api/v1/apps/$APP_ID/fcp
   ```
10. Toggle the anonymous setting off, save, and POST without a token from a
    terminal:
    ```bash
    curl -s -i -X POST http://localhost:9300/api/v1/apps/$APP_ID/fcp \
      -H 'Content-Type: text/plain' --data 'hi'
    ```

## Expected Result

- Step 3: every field listed renders without lint or runtime errors.
- Step 4: the bearer token input is non-empty and looks random.
- Step 7: the channel saves; navigation returns to the FCP channel's edit view.
- Step 8: every UI assertion matches.
- Step 9: HTTP `200` with `Content-Type: text/markdown; charset=utf-8`. Body
  is **exactly** the custom handshake from Test Data — no auto-generated
  description appears.
- Step 10: HTTP `401`. Body is Markdown and mentions both
  `Authorization: Bearer` and `X-Everruns-FCP-Token`. The configured bearer
  token is **never** echoed in the response body.
