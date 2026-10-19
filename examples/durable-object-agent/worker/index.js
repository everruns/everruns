import { fetchModel } from "./provider.js";
import wasm from "../pkg/everruns_object_agent_bg.wasm";
import { initSync, binding, run, request as sessionRequest } from "../pkg/everruns_object_agent.js";
import worker, {
  SessionAuthority as Base,
  authenticate, address, readJson, failure,
} from "../../../crates/durable-object/worker/src/index.js";
import { runActivation } from "../../../crates/durable-object/worker/src/activation.js";
import { objectPort } from "../../../crates/durable-object/worker/src/isolate.js";
import { sessionApi } from "../../../crates/durable-object/worker/src/session-api.js";
initSync({ module: wasm });

export class SessionAuthority extends Base {
  async fetch(request) {
    try {
      await authenticate(request, this.env.AUTHORITY_TOKEN);
      const scope = address(request, this.env);
      if (scope.operation !== "engine") return super.fetch(request);
      const body = await readJson(request);
      const reply = JSON.parse(await sessionRequest(scope.tenant, scope.sessionId,
        this.port({tenant:scope.tenant,session_id:scope.sessionId}),
        (raw,signal)=>this.model(raw,signal),this.env.MODEL ?? "gpt-6-luna",
        body.operation,JSON.stringify(body.body ?? {})));
      return Response.json(reply.value,{status:reply.status});
    } catch(error) { return this.failure(error); }
  }
  // Hosts may attach server-side diagnostics while keeping public failures redacted.
  failure(error) { return failure(error); }
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
export default {
  fetch(request,env) {
    if (new URL(request.url).pathname.startsWith("/v1/")) return sessionApi(request,env);
    return worker.fetch(request,env);
  },
};
