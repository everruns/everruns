import { DurableObject } from "cloudflare:workers";
import { Authority } from "./authority.js";
import { runAlarm } from "./activation.js";
import { authenticate, address, readJson, failure } from "./http.js";
import { requireValue } from "./storage.js";
export { authenticate, address, readJson, failure } from "./http.js";

export class SessionAuthority extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.authority = new Authority(ctx.storage);
    this.authority.journal.onChange = () => {
      for (const socket of this.ctx.getWebSockets("events")) {
        try { socket.send("changed"); } catch { try { socket.close(1011, "Reconnect to replay"); } catch {} }
      }
    };
  }

  async fetch(request) {
    try {
      await authenticate(request, this.env.AUTHORITY_TOKEN);
      const { tenant, sessionId, operation } = address(request, this.env);
      if (operation === "watch") {
        requireValue(request.headers.get("upgrade")?.toLowerCase() === "websocket", 426, "WebSocket required");
        const state = this.authority.journal.read();
        requireValue(state.tenant === tenant && state.session_id === sessionId, 404, "session not found");
        requireValue(this.ctx.getWebSockets("events").length < 64, 429, "observer capacity reached");
        const [client, server] = Object.values(new WebSocketPair());
        this.ctx.acceptWebSocket(server, ["events"]);
        return new Response(null, { status: 101, webSocket: client });
      }
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

  webSocketMessage(socket) { socket.close(1008, "Read-only event notifications"); }
  webSocketClose(socket, code) { socket.close(code, ""); }
  webSocketError(socket) { socket.close(1011, "Reconnect to replay"); }
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
