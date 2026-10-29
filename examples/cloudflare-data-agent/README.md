# Cloudflare D1 data agent

A customer-hosted JavaScript agent, executed by Cloudflare Workers, with a private
D1 sales database. Everruns owns Agent/session identity, intake, actor leases,
canonical events, Playground, Trace and channel delivery. The Worker owns model
credentials, agent logic and database access. This example calls a real
`gpt-6.1-sol` model; it does not require a Rust binary or a persistent poller.

Requires an Everruns installation with `EXTERNAL_AGENTS_ENABLED=true`, encryption
configured, and an outbound network policy that permits your HTTPS endpoint. This is opt-in experimental functionality;
verify that your Cloud installation has enabled it before deploying.

## Deploy

```bash
npm install
npx wrangler login
npx wrangler d1 create everruns-sales
```

Put the returned database ID in `wrangler.jsonc`, then:

```bash
npx wrangler d1 execute everruns-sales --remote --file schema.sql
npx wrangler secret put OPENAI_API_KEY
# Generate a unique random secret, at least 32 characters. Keep it in your
# password manager; enter the same secret into Everruns during registration.
npx wrangler secret put EVERRUNS_ENDPOINT_SECRET
npm run deploy
```

The provider key and D1 binding stay in your account. Never put an Everruns PAT,
CP database URL, fleet token, or Slack bot token in this Worker.

## Register and invoke

Create a normal Agent in Everruns. On its Agent page choose **Configure execution**,
enter your deployed HTTPS URL and endpoint secret, then **Verify and connect**.
The endpoint must pass an authenticated challenge before the CP saves a binding.
Changing a binding requires no active turns. The account needs permission to
manage dangerous Agent settings.

Or use `POST /api/v1/agents/{agent_id}/external-worker` with your ordinary user
credential and JSON:

```json
{
  "endpoint_url": "https://everruns-data-agent.YOUR-SUBDOMAIN.workers.dev/",
  "endpoint_secret": "YOUR_UNIQUE_RANDOM_ENDPOINT_SECRET"
}
```

Use the Agent's **Test in Playground** action and ask:

> What are total sales and order count for north? Include the exact total cents.

Expect **27278 cents, 2 orders**. Open the session's Trace to inspect model calls,
`sales_summary` arguments/results and usage. Follow-up messages replay completed
conversation history from the CP journal. Editor model/prompt/tool settings do
not configure this external implementation.

Set `EVERRUNS_URL` to the CP origin (for example,
`https://app.everruns.com`), without `/api`. `EVERRUNS_TOKEN` is your ordinary
user PAT, used only by your terminal; never install it in the Worker.

The ordinary session API also works:

```bash
curl "$EVERRUNS_URL/api/v1/sessions" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"agent_id":"agent_YOUR_ID","source":"playground"}'
# Substitute the returned session ID:
curl "$EVERRUNS_URL/api/v1/sessions/session_YOUR_ID/messages" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"What are north sales? Include exact cents and order count."}]}}'
```

`GET` the external-worker resource for status; `DELETE` disables it. Disabled
bindings stay bound, so pending input cannot fall back to the built-in runtime.
Re-register the endpoint to rotate its secret or re-enable it.

## Slack

On that same Agent's **Integrations** tab, add and publish Slack using
[the Slack guide](https://everruns.com/docs/how-to/publish-to-slack/). Use **Direct**
communication. Mention the Agent in a Slack thread with the north-sales question.
Slack intake creates/binds the normal session; CP dispatches to the Worker;
`output.message.completed` is delivered by CP to the thread. Follow-up messages
use the same conversation. The Worker needs no Slack token or webhook.

You can create the draft Slack channel through the ordinary API too, then finish
installation/publishing on the Agent's Integrations page:

```bash
curl "$EVERRUNS_URL/api/v1/agents/agent_YOUR_ID/channels" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"channel_type":"slack","channel_config":{"session_strategy":"per_thread","response_policy":"mentions_only"},"enabled":true}'
```

This registers Slack intake on the existing Agent. Its external-worker binding
selects execution for those sessions as well as Playground sessions.

The sample supports basic text replies. Native Slack actions, Explicit
communication, approvals, delegation and attachments are outside this example.
Authorize access to private data per Slack user in your implementation before
using sensitive production data. Endpoint authentication only proves the CP sent
the invocation; it does not implement your application's data authorization.

## Protocol and limits

`src/protocol.js` implements the `everruns-worker/v1` request contract. The CP
POSTs signed JSON; the signature is HMAC-SHA256 of the exact body bytes in
`X-Everruns-Signature`. Both verification and invocation requests carry an
`issued_at` timestamp. The Worker rejects signatures older/newer than 60 seconds.
Verification returns an HMAC of a fresh challenge, not a public nonce echo.

Invocations contain a stable `invocation_id`, session ID, current input,
`input_sequence`, and bounded journal. Return one canonical execution event JSON
object per newline. `eventStream` supplies turn/input correlation. The adapter
pins the session and input, validates each event using the existing custom-worker
service, then publishes through ordinary EventService. Reported provider IDs are
stripped. The response stream grants no callback or management authority.

The adapter renews actor leases and closes the request on cancellation/revocation.
The current implementation bounds each invocation to 120 seconds, context/history
to 2000 events / 8 MiB, the invocation body and response to 16 MiB, and each event to 1 MiB. Admission allows at most four active HTTP invocations
per organization across the fleet, and eight per CP process; each turn accepts at
most 2048 events. Model responses are currently
reported as completed generations, not token deltas. An interrupted or invalid
stream fails the turn and requires an explicit new message; it is not replayed
automatically. The CP cannot roll back remote effects or guarantee that remote
code stops when a connection closes. Write-capable tools need durable deduplication
by invocation ID and their own interruption policy. This example only reads D1.

## Tests and live smoke

```bash
npm test
# Against a running CP and a deployed Worker with the seeded D1 database:
python3 smoke.py --cp "$EVERRUNS_URL" --endpoint "$WORKER_URL"
```

The smoke reads `EVERRUNS_TOKEN` and `EVERRUNS_ENDPOINT_SECRET` from its environment.
Pass `--org org_...` if your account belongs to multiple organizations.
It creates a disposable Agent, verifies endpoint registration, invokes the real
model and private database, checks durable events/trace provenance/tool results,
checks follow-up history, disables the endpoint and confirms queued work cannot
run, then revokes its binding. It prints IDs and assertions, never credentials.
It does not post messages to Slack; test that separately in a designated channel.
