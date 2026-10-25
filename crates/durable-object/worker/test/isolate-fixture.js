// Test-only fault injection and scripted provider. The runnable example uses a real model.
import worker, {
  SessionAuthority as App,
} from "../../../../examples/durable-object-agent/worker/index.js";
export class SessionAuthority extends App {
  constructor(ctx, env) {
    super(ctx, env);
    this.qualificationContext = ctx;
  }
  failure(error) {
    console.error("Synthetic qualification failure", error);
    return super.failure(error);
  }
  async fetch(request) {
    if (new URL(request.url).pathname.endsWith("/evict")) {
      const status = new URL(request.url);
      status.pathname = status.pathname.replace(/evict$/, "status");
      const verified = await super.fetch(
        new Request(status, {
          method: "POST",
          headers: request.headers,
          body: "{}",
        }),
      );
      if (!verified.ok) return verified;
      this.qualificationContext.abort("qualification: lose the isolate after tool effect");
    }
    return super.fetch(request);
  }
  port(activation) {
    const rpc = super.port(activation);
    return async (raw) => {
      const reply = await rpc(raw);
      const { operation, body } = JSON.parse(raw);
      if (
        operation === "store" &&
        body.kind === "value" &&
        body.operation === "set" &&
        body.key === "counter" &&
        body.value === "1"
      ) {
        // The side effect has committed, but the tool completion is not recorded.
        await new Promise(() => {});
      }
      return reply;
    };
  }
  async model(raw) {
    const request = JSON.parse(JSON.parse(raw).body);
    const afterTool = request.messages.at(-1)?.role === "tool";
    const message = afterTool
      ? { role: "assistant", content: "Recovered." }
      : {
          role: "assistant",
          content: null,
          tool_calls: [
            {
              id: "call_counter",
              type: "function",
              function: { name: "bump_counter", arguments: "{}" },
            },
          ],
        };
    const delta = afterTool
      ? { role: "assistant", content: message.content }
      : {
          role: "assistant",
          tool_calls: message.tool_calls.map((call) => ({ ...call, index: 0 })),
        };
    const chunks = [
      {
        id: "qualification-completion",
        object: "chat.completion.chunk",
        model: request.model,
        created: Math.floor(Date.now() / 1000),
        choices: [{ index: 0, delta, finish_reason: null }],
      },
      {
        id: "qualification-completion",
        object: "chat.completion.chunk",
        model: request.model,
        created: Math.floor(Date.now() / 1000),
        choices: [{ index: 0, delta: {}, finish_reason: afterTool ? "stop" : "tool_calls" }],
        usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
      },
    ];
    return new Response(
      chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join("") + "data: [DONE]\n\n",
      { headers: { "content-type": "text/event-stream" } },
    );
  }
}
export default {
  async fetch(request,env) {
    // Test-only independent client credential. The temporary deployment uses a
    // fresh authority token, so its public API credential is also fresh.
    const hash = await crypto.subtle.digest("SHA-256",new TextEncoder().encode(`${env.AUTHORITY_TOKEN}:api`));
    const API_TOKEN = [...new Uint8Array(hash)].map(byte=>byte.toString(16).padStart(2,"0")).join("");
    return worker.fetch(request,{...env,API_TOKEN});
  },
};
