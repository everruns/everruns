import { fireEvent, render, screen } from "@testing-library/react";
import { SessionForkButton } from "@/components/session/session-fork-button";

const push = jest.fn();
jest.mock("next/navigation", () => ({ useRouter: () => ({ push }) }));
jest.mock("@/hooks/use-sessions", () => ({
  useForkSession: () => ({
    isPending: false,
    mutate: (_request: unknown, callbacks: { onSuccess: (session: { id: string }) => void }) =>
      callbacks.onSuccess({ id: "session_forked" }),
  }),
}));
const props = {
  sessionId: "session_recording",
  sessionTitle: "Recording",
  sessionTags: [],
  platformChat: true,
};
beforeEach(() => push.mockClear());
it("lets the MCP host open a fork instead of navigating the credentialless frame into browser Chat", () => {
  const open = jest.fn();
  render(<SessionForkButton {...props} onForked={open} />);
  fireEvent.click(screen.getByRole("button"));
  expect(open).toHaveBeenCalledWith("session_forked");
  expect(push).not.toHaveBeenCalled();
});
it("keeps ordinary browser navigation when no embedding callback is supplied", () => {
  render(<SessionForkButton {...props} />);
  fireEvent.click(screen.getByRole("button"));
  expect(push).toHaveBeenCalledWith("/chats/session_forked");
});
