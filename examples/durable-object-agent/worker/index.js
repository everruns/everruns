import { fetchModel } from "./provider.js";
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
        (raw, signal) => this.model(raw, signal),
        this.env.MODEL ?? "gpt-6-luna",
      );
      return 200;
    });
  }

  port(activation) {
    return objectPort(this.authority, activation);
  }

  async model(raw, signal) {
    return fetchModel(this.env, raw, signal);
  }
}
export default worker;
