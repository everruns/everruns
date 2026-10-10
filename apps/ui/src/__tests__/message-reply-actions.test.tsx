import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MessageReplyActions } from "@/components/chat/message-reply-actions";
import { listMessageFeedback, setMessageFeedback } from "@/lib/api/message-feedback";
import { forkSession } from "@/lib/api/sessions";

const push = jest.fn();
jest.mock("next/navigation", () => ({ useRouter: () => ({ push }) }));
jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en", t: (key: string) => key }),
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" }, isLoading: false }),
}));
jest.mock("@/lib/api/message-feedback", () => ({
  listMessageFeedback: jest.fn(),
  setMessageFeedback: jest.fn(),
}));
jest.mock("@/lib/api/sessions", () => ({
  ...jest.requireActual("@/lib/api/sessions"),
  forkSession: jest.fn(),
}));

const listMock = listMessageFeedback as jest.MockedFunction<typeof listMessageFeedback>;
const setMock = setMessageFeedback as jest.MockedFunction<typeof setMessageFeedback>;
const forkMock = forkSession as jest.MockedFunction<typeof forkSession>;

function renderActions(sessionActive = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MessageReplyActions sessionId="session_1" messageId="msg_2" sessionActive={sessionActive} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  setMock.mockResolvedValue({
    message_id: "msg_2",
    rating: "good",
    comment: null,
    updated_at: new Date().toISOString(),
  });
});

it("shows the saved rating and clears it when pressed again", async () => {
  listMock.mockResolvedValue([
    { message_id: "msg_2", rating: "good", comment: null, updated_at: "2026-10-10T00:00:00Z" },
  ]);
  renderActions();

  const good = screen.getByTestId("message-feedback-good");
  await waitFor(() => expect(good).toHaveAttribute("aria-pressed", "true"));
  expect(screen.getByTestId("message-feedback-bad")).toHaveAttribute("aria-pressed", "false");

  fireEvent.click(good);
  await waitFor(() => expect(setMock).toHaveBeenCalledWith("session_1", "msg_2", null));
});

it("rates a reply bad", async () => {
  // Unrated at first; the refetch after saving returns the stored rating.
  listMock
    .mockResolvedValueOnce([])
    .mockResolvedValue([
      { message_id: "msg_2", rating: "bad", comment: null, updated_at: "2026-10-10T00:00:00Z" },
    ]);
  renderActions();
  await waitFor(() => expect(listMock).toHaveBeenCalledTimes(1));

  fireEvent.click(screen.getByTestId("message-feedback-bad"));
  await waitFor(() => expect(setMock).toHaveBeenCalledWith("session_1", "msg_2", "bad"));
  await waitFor(() =>
    expect(screen.getByTestId("message-feedback-bad")).toHaveAttribute("aria-pressed", "true"),
  );
});

it("branches into a new chat up to this message", async () => {
  listMock.mockResolvedValue([]);
  forkMock.mockResolvedValue({ id: "session_2" } as Awaited<ReturnType<typeof forkSession>>);
  renderActions();

  fireEvent.click(screen.getByTestId("message-branch"));
  await waitFor(() => expect(push).toHaveBeenCalledWith("/chats/session_2"));
  expect(forkMock).toHaveBeenCalledWith("session_1", { up_to_message_id: "msg_2" });
});

it("does not branch while a turn is running", () => {
  listMock.mockResolvedValue([]);
  renderActions(true);
  expect(screen.getByTestId("message-branch")).toBeDisabled();
});
