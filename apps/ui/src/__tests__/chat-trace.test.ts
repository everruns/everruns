import { buildTraceUrl } from "@/lib/chat-trace";

describe("buildTraceUrl", () => {
  const params = { responseId: "resp_123", sessionId: "session_123", turnId: "turn_123" };

  it("builds public HTTPS trace links", () => {
    expect(buildTraceUrl("https://openrouter.ai/logs?id={response_id}", params)).toBe(
      "https://openrouter.ai/logs?id=resp_123",
    );
  });

  it("rejects plaintext and internal trace link targets", () => {
    for (const template of [
      "http://logs.example.com/{response_id}",
      "https://127.0.0.1/{response_id}",
      "https://10.0.0.1/{response_id}",
      "https://169.254.169.254/{response_id}",
      "https://localhost/{response_id}",
      "https://router.local/{response_id}",
    ]) {
      expect(buildTraceUrl(template, params)).toBeNull();
    }
  });
});
