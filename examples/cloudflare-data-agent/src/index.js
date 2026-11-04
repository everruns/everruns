// Worker entry point. Everruns POSTs signed JSON here: a one-time
// `verify` challenge when the endpoint is registered, then one `invoke`
// per message.
import { execute } from "./agent.js";
import { authenticate, eventStream, hmac } from "./protocol.js";

export default {
  async fetch(request, env, ctx) {
    const secret = env.EVERRUNS_ENDPOINT_SECRET;
    let body;
    try {
      body = await authenticate(request, secret);
    } catch {
      return new Response("Unauthorized", { status: 401 });
    }

    if (body.type === "verify" && typeof body.challenge === "string") {
      const challenge = new TextEncoder().encode(body.challenge);
      return Response.json({ proof: await hmac(secret, challenge) });
    }

    const validInvocation =
      body.type === "invoke" &&
      /^message_[a-f0-9]{32}$/.test(body.invocation_id ?? "") &&
      body.input;
    if (!validInvocation) {
      return new Response("Invalid invocation", { status: 400 });
    }

    // This example only reads data. Side-effecting agents must durably
    // dedupe invocation_id themselves; a timestamp/signature is not
    // exactly-once delivery.
    const { response, task } = eventStream(body, (emit, turn, signal) =>
      execute(body, env, emit, turn, fetch, signal),
    );
    ctx.waitUntil(task);
    return response;
  },
};
