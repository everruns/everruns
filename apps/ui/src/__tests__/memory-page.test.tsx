jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { role: "owner" } }),
}));
import { fireEvent, render, screen, within } from "@testing-library/react";
import MemoryPage from "@/app/(main)/memory/page";
import { memoryStore } from "@/test-utils/memory-fixtures";

const mockUseMemoryStores = jest.fn();
const mockCreate = jest.fn();

jest.mock("@/hooks", () => ({
  useMemoryStores: () => mockUseMemoryStores(),
  useCreateMemory: () => ({ mutateAsync: mockCreate, isPending: false }),
  usePageTitle: () => undefined,
}));

const stores = [
  memoryStore({
    id: "mem_00000000000000000000000000000001",
    name: "org",
    store: "org",
    mount_path: "/memory/org",
    about: "The organization's memory; every agent with Memory reads it.",
    blocked_count: 0,
  }),
  memoryStore({ id: "mem_00000000000000000000000000000002" }),
  memoryStore({
    id: "mem_00000000000000000000000000000003",
    scope: "team",
    name: "team-memory-x",
    store: "team",
    owner_name: "Housekeeping",
    mount_path: "/memory/team",
    blocked_count: 2,
  }),
  memoryStore({
    id: "mem_00000000000000000000000000000004",
    scope: "agent",
    name: "agent-memory-x",
    store: "agent",
    owner_name: "Developer",
    mount_path: "/memory/agent",
    blocked_count: 0,
  }),
  memoryStore({
    id: "mem_00000000000000000000000000000005",
    scope: "user",
    name: "user-memory-x",
    store: "user",
    mount_path: "/memory/user",
    blocked_count: 0,
  }),
];

describe("Memory page", () => {
  beforeEach(() => {
    mockUseMemoryStores.mockReturnValue({ data: stores, isLoading: false, error: null });
    mockCreate.mockReset();
  });

  it("lists every store by owner, with where agents find it", () => {
    render(<MemoryPage />);
    expect(screen.getAllByText("Organization").length).toBeGreaterThan(0);
    expect(screen.getByText("Housekeeping team")).toBeTruthy();
    expect(screen.getByText("Developer")).toBeTruthy();
    expect(screen.getByText("Your memory")).toBeTruthy();
    expect(screen.getByText("/memory/everruns-maintenance")).toBeTruthy();
    expect(screen.getByText("2 blocked")).toBeTruthy();
  });

  it("filters to the stores agents share", () => {
    render(<MemoryPage />);
    fireEvent.click(screen.getByRole("tab", { name: "Shared" }));
    expect(screen.getByText("Housekeeping team")).toBeTruthy();
    expect(screen.queryByText("Developer")).toBeNull();
    expect(screen.queryByText("Your memory")).toBeNull();
  });

  it("creates a named memory and refuses names agents cannot use", async () => {
    render(<MemoryPage />);
    fireEvent.click(screen.getAllByRole("button", { name: /New named memory/ })[0]);
    const dialog = screen.getByRole("dialog");
    const name = within(dialog).getByLabelText("Name");
    fireEvent.change(name, { target: { value: "Team Notes" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(await within(dialog).findByText(/lower-case letters/)).toBeTruthy();
    expect(mockCreate).not.toHaveBeenCalled();

    fireEvent.change(name, { target: { value: "team" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(await within(dialog).findByText(/built-in store/)).toBeTruthy();

    fireEvent.change(name, { target: { value: "research-notes" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    await screen.findByText("New named memory", { selector: "button *, button" });
    expect(mockCreate).toHaveBeenCalledWith({ name: "research-notes", description: undefined });
  });
});
