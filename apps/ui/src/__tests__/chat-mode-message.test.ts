import { sendChatMessage } from "@/lib/api/messages";
import { api } from "@/lib/api/client";
import { chatPageRoute } from "@/lib/chat-page-context";
jest.mock("@/lib/api/client", () => ({
  api: { post: jest.fn().mockResolvedValue({ data: { id: "message_1" } }) },
}));

it("shares page tabs and sheets without forwarding arbitrary callback parameters", () => {
  expect(
    chatPageRoute(
      "/agents/agent_1",
      new URLSearchParams("access_token=private&tab=preview&sheet=history&mode=edit&code=private"),
    ),
  ).toBe("/agents/agent_1?tab=preview&sheet=history&mode=edit");
});

it("sends page context and mouse actions as request metadata, keeping authored text intact", async () => {
  const metadata = {
    page_context: { route: "/agents/agent_1", entity_id: "agent_1", label: "Support" },
    page_actions: [{ label: "Saved manual edits" }],
  };
  await sendChatMessage("session_1", {
    clientMessageId: "client_1",
    text: "Improve this",
    metadata,
  });
  expect(api.post).toHaveBeenCalledWith(
    "/v1/sessions/session_1/messages",
    expect.objectContaining({
      metadata,
      client_message_id: "client_1",
      message: { role: "user", content: [{ type: "text", text: "Improve this" }] },
    }),
    undefined,
  );
});
