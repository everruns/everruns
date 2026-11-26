import { addRoomMember } from "@/lib/api/rooms";
import { api } from "@/lib/api/client";

jest.mock("@/lib/api/client", () => ({ api: { post: jest.fn() } }));

it("sends the flat Room member command accepted by the server", async () => {
  const member = { kind: "agent" as const, agent_id: "agent_editor", wake: "mentions" as const };
  (api.post as jest.Mock).mockResolvedValue({ data: { id: "rmem_editor" } });
  await addRoomMember("room_editorial", member);
  expect(api.post).toHaveBeenCalledWith("/v1/platform/rooms/room_editorial/members", member);
});
