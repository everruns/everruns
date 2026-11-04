// Test-only provider faults and inspection. The example uses a real provider.
import worker, { SessionAuthority as App } from "../../../../examples/durable-object-agent/worker/index.js";

export class SessionAuthority extends App {
  constructor(ctx, env) {
    super(ctx, env);
    this.instance = crypto.randomUUID();
  }
  async fetch(request) {
    const url = new URL(request.url);
    const operation = url.pathname.split("/").at(-1);
    if (["probe", "hold", "clear"].includes(operation)) {
      url.pathname = url.pathname.replace(/[^/]+$/, "status");
      const checked = await super.fetch(new Request(url, { method: "POST", headers: request.headers, body: "{}" }));
      if (!checked.ok) return checked;
      if (operation === "hold") this.pendingTimer = setTimeout(() => {}, 60000);
      if (operation === "clear") clearTimeout(this.pendingTimer);
      return Response.json({ instance: this.instance, state: this.authority.journal.read() });
    }
    return super.fetch(request);
  }
  async model(raw) {
    const sql = this.ctx.storage.sql;
    sql.exec("CREATE TABLE IF NOT EXISTS test_calls (n INTEGER NOT NULL)");
    const count = sql.exec("SELECT n FROM test_calls").toArray()[0]?.n ?? 0;
    sql.exec("DELETE FROM test_calls");
    sql.exec("INSERT INTO test_calls VALUES (?)", count + 1);
    if (!count) return Response.json({ error: { message: "temporary rate limit", type: "rate_limit_error" } }, { status: 429, headers: { "retry-after": "12" } });
    const request = JSON.parse(JSON.parse(raw).body);
    const chunk = { id: "hibernation", object: "chat.completion.chunk", created: 0, model: request.model,
      choices: [{ index: 0, delta: { role: "assistant", content: "Resumed from a durable sleep." }, finish_reason: "stop" }] };
    return new Response(`data: ${JSON.stringify(chunk)}\n\ndata: [DONE]\n\n`, { headers: { "content-type": "text/event-stream" } });
  }
}
export default worker;
