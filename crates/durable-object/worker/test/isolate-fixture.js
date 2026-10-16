// Test-only fault injection and scripted provider. The runnable example uses a real model.
import worker, {
  SessionAuthority as App,
} from "../../../../examples/durable-object-agent/worker/index.js";
export class SessionAuthority extends App {
  constructor(ctx, env) {
    super(ctx, env);
    this.qualificationContext = ctx;
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
    const request = JSON.parse(raw);
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
    return JSON.stringify({
      status: 200,
      body: {
        id: "qualification-completion",
        choices: [{ message, finish_reason: afterTool ? "stop" : "tool_calls" }],
        usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
      },
    });
  }
}
export default worker;
