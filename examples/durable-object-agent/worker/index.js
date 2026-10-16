import wasm from "../pkg/everruns_object_agent_bg.wasm";
import { initSync, binding, run } from "../pkg/everruns_object_agent.js";
import worker, {
  SessionAuthority as Base,
} from "../../../crates/durable-object/worker/src/index.js";
import { runActivation } from "../../../crates/durable-object/worker/src/activation.js";
import { objectPort } from "../../../crates/durable-object/worker/src/isolate.js";
initSync({ module: wasm });

export class SessionAuthority extends Base {
  async alarm() {
    return runActivation(this.authority, async (activation) => {
      const expected = JSON.parse(binding());
      if (
        activation.binding.agent !== expected.agent ||
        activation.binding.build_id !== expected.build_id
      )
        return 409;
      const rpc = this.port(activation);
      await run(
        activation.tenant,
        activation.session_id,
        rpc,
        (raw) => this.model(raw),
        this.env.MODEL ?? "gpt-6-luna",
      );
      return 200;
    });
  }

  port(activation) {
    return objectPort(this.authority, activation);
  }

  async model(raw) {
    const url = new URL(
      `${(this.env.OPENAI_BASE_URL ?? "https://api.openai.com/v1").replace(/\/$/, "")}/chat/completions`,
    );
    if (
      !(
        url.protocol === "https:" ||
        (url.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname))
      ) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    )
      throw new Error("invalid provider endpoint");
    if (!this.env.OPENAI_API_KEY) throw new Error("provider credential missing");
    const response = await fetch(url, {
      method: "POST",
      redirect: "manual",
      signal: AbortSignal.timeout(60000),
      headers: {
        authorization: `Bearer ${this.env.OPENAI_API_KEY}`,
        "content-type": "application/json",
      },
      body: raw,
    });
    const reader = response.body?.getReader();
    const chunks = [];
    let size = 0;
    if (reader) {
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > 3 * 1024 * 1024) throw new Error("provider reply exceeds limit");
          chunks.push(value);
        }
      } catch (error) {
        await reader.cancel().catch(() => {});
        throw error;
      }
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    const body = response.ok ? JSON.parse(new TextDecoder().decode(bytes)) : null;
    return JSON.stringify({ status: response.status, body });
  }
}
export default worker;
