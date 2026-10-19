import { DurableObject } from "cloudflare:workers";
import "./rust.js";
import { Authority } from "./authority.js";
import { runAlarm } from "./activation.js";
import { authenticate, address, readJson, failure } from "./http.js";
export { authenticate, address, readJson, failure } from "./http.js";

export class SessionAuthority extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.authority = new Authority(ctx.storage);
  }

  async fetch(request) {
    try {
      await authenticate(request, this.env.AUTHORITY_TOKEN);
      const { tenant, sessionId, operation } = address(request, this.env);
      return Response.json(
        await this.authority.handle(
          operation,
          await readJson(request),
          sessionId,
          tenant,
        ),
      );
    } catch (error) {
      return failure(error);
    }
  }

  async alarm() {
    return runAlarm(this.authority, this.env);
  }
}

export default {
  async fetch(request, env) {
    try {
      await authenticate(request, env.AUTHORITY_TOKEN);
      const { tenant, sessionId } = address(request, env);
      const stub = env.SESSIONS.get(
        env.SESSIONS.idFromName(`${tenant}:${sessionId}`),
      );
      return await stub.fetch(request);
    } catch (error) {
      return failure(error);
    }
  },
};
