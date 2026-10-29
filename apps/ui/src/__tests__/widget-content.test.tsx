import { render } from "@testing-library/react";
import {
  SessionContext,
  type SessionContextValue,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import { WidgetContent } from "@/components/chat/widget-content";

const mockWidgets = jest.fn();
jest.mock(
  "next/dynamic",
  () => () =>
    function Widgets(props: unknown) {
      mockWidgets(props);
      return null;
    },
);
jest.mock("next/navigation", () => ({ useParams: () => ({}) }));
jest.mock("@/providers/locale-provider", () => ({ useUiTranslations: () => (key: string) => key }));
jest.mock("@/components/chat/streamdown-message", () => ({ StreamdownMessage: () => null }));
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionContext: require("react").createContext(null),
}));
const mutateAsync = jest.fn().mockResolvedValue({});
function renderWidget(overrides: Partial<SessionContextValue> = {}) {
  const context = {
    sessionId: "session_from_context",
    sendMessage: { mutateAsync, isPending: false },
    ...overrides,
  } as unknown as SessionContextValue;
  return render(
    <SessionContext.Provider value={context}>
      <WidgetContent text={"```a2ui\n{}\n```"} />
    </SessionContext.Provider>,
  );
}
beforeEach(() => {
  mockWidgets.mockClear();
  mutateAsync.mockClear();
});

test("chat URLs without sessionId still send to the session in context", async () => {
  renderWidget();
  await mockWidgets.mock.calls.at(-1)![0].onMessage("Review the release");
  expect(mutateAsync).toHaveBeenCalledWith({
    sessionId: "session_from_context",
    content: "Review the release",
  });
});
test("archived conversations provide no action callback", () => {
  renderWidget({
    session: { archived_at: "2026-10-28T12:00:00Z" } as SessionContextValue["session"],
  });
  expect(mockWidgets.mock.calls.at(-1)![0].onMessage).toBeUndefined();
});
test("active sessions disable widget execution", () => {
  renderWidget({ isActive: true });
  expect(mockWidgets.mock.calls.at(-1)![0].streaming).toBe(true);
});
test("standalone previews without a session are read-only", () => {
  render(<WidgetContent text={"```a2ui\n{}\n```"} />);
  expect(mockWidgets.mock.calls.at(-1)![0].onMessage).toBeUndefined();
});
