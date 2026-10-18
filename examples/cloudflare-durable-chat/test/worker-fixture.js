// Test-only fault injection around the production objects. Never deployed.
import worker, {
  ChatRoom as Room,
  ChatSession as Session,
} from "../worker/index.js";

export class ChatRoom extends Room {
  async fetch(request) {
    if (request.method === "POST") {
      const body = await request
        .clone()
        .json()
        .catch(() => null);
      if (body?.type === "__seed_legacy") {
        this.ctx.storage.sql.exec(
          "INSERT INTO chat(id,payload) VALUES(1,?)",
          JSON.stringify(body.data),
        );
        return Response.json({ seeded: true });
      }
      if (body?.type === "__legacy_backup") {
        const row = this.ctx.storage.sql
          .exec("SELECT payload FROM chat_migration_backup WHERE id=1")
          .toArray()[0];
        return Response.json(JSON.parse(row.payload));
      }
    }
    return super.fetch(request);
  }
}

export class ChatSession extends Session {
  async fetch(request) {
    const body = await request
      .clone()
      .json()
      .catch(() => null);
    const response = await super.fetch(request);
    if (
      body?.operation === "send" &&
      response.ok &&
      !(await this.ctx.storage.get("test_dropped_admission_ack"))
    ) {
      await this.ctx.storage.put("test_dropped_admission_ack", true);
      // Admission committed; its caller cannot know that it succeeded.
      return Response.json(
        { error: "test: lost admission acknowledgement" },
        { status: 503 },
      );
    }
    return response;
  }
}
export default worker;
