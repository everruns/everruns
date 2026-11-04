import { PageChatProvider } from "@/test-utils/page-chat-provider";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import ProfilePage from "@/app/(main)/settings/profile/page";
import { type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";

let mockUser = { id: "user_fixture", name: "Alex Northstar", email: "alex@example.test" };
const mockSave = jest.fn(async ({ name }: { name: string }) => {
  mockUser = { ...mockUser, name };
});
jest.mock("@/providers/auth-provider", () => ({ useAuth: () => ({ user: mockUser }) }));
jest.mock("@/hooks/use-auth", () => ({
  useUpdateProfile: () => ({ mutateAsync: mockSave, isPending: false }),
}));

beforeEach(() => {
  mockSave.mockClear();
  mockUser = { id: "user_fixture", name: "Alex Northstar", email: "alex@example.test" };
});
beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});

it("stages the profile name and preserves read-only identity fields until normal Save", async () => {
  let adapter: PageDraftAdapter;
  const mode: ChatModeState = {
    page: { route: "/settings/profile", label: "Profile" },
    pageContainer: { current: null },
    open: true,
    setOpen: () => {},
    actions: [],
    recordAction: () => {},
    registerPage: (_page, value) => {
      if (value) adapter = value;
      return () => {};
    },
  };
  render(
    <PageChatProvider value={mode}>
      <ProfilePage />
    </PageChatProvider>,
  );
  const before = adapter!.read(adapter!.binding!);
  expect(before.values).toEqual({ name: "Alex Northstar" });
  expect(() =>
    adapter!.patch(before.binding, before.revision, { email: "other@example.test" }),
  ).toThrow(/unsupported/);
  act(() => {
    adapter!.patch(before.binding, before.revision, { name: "Alex Review" });
  });
  expect(screen.getByLabelText("Full Name")).toHaveValue("Alex Review");
  expect(screen.getByLabelText("Email")).toHaveValue("alex@example.test");
  expect(mockSave).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mockSave).toHaveBeenCalledWith({ name: "Alex Review" }));
});

it("retains the ordinary profile controls when Chat mode is off", () => {
  render(<ProfilePage />);
  fireEvent.change(screen.getByLabelText("Full Name"), { target: { value: "Manual name" } });
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: "Discard" })).not.toBeInTheDocument();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
