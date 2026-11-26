import { act, fireEvent, render, screen } from "@testing-library/react";
import { Suspense } from "react";
import MemoryDetailPage from "@/app/(main)/memory/[memoryId]/page";
import { groupMemoryEntries } from "@/components/memory/memory-store-labels";
import { memoryEntry, memoryStore } from "./fixtures/memory";

const mockUseMemoryStores = jest.fn();
const mockUseMemoryEntries = jest.fn();
const mockUseMemoryHistory = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: jest.fn(), push: jest.fn() }),
  usePathname: () => "/",
  useSearchParams: () => new URLSearchParams(),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1", role: "owner" }, isLoading: false }),
}));

jest.mock("@/hooks", () => ({
  useMemoryStores: () => mockUseMemoryStores(),
  useMemoryEntries: (...args: unknown[]) => mockUseMemoryEntries(...args),
  useMemoryHistory: (...args: unknown[]) => mockUseMemoryHistory(...args),
  useUpdateMemory: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useArchiveMemory: () => ({ mutateAsync: jest.fn(), isPending: false }),
  usePageTitle: () => undefined,
}));

const store = memoryStore({
  scope: "team",
  store: "team",
  owner_name: "Housekeeping",
  mount_path: "/memory/team",
});

const entries = [
  memoryEntry(),
  memoryEntry({
    path: "/blocked/everruns-5530.md",
    kind: "blocked",
    subject: "everruns#5530",
    statement: "Waiting on Mike to allowlist the package",
    quote: null,
  }),
  memoryEntry({
    path: "/blocked/old.md",
    kind: "blocked",
    subject: "old",
    statement: "Waiting on CI",
    resolved_at: "2026-11-26T05:00:00Z",
    resolution: "green",
  }),
  memoryEntry({
    path: "/proposals/rules/merges.md",
    kind: "rule",
    subject: "merges",
    statement: "Squash only",
    proposed: true,
  }),
];

async function renderPage(memoryId = store.id) {
  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <MemoryDetailPage params={Promise.resolve({ memoryId })} />
      </Suspense>,
    );
  });
}

describe("Memory detail page", () => {
  beforeEach(() => {
    mockUseMemoryStores.mockReturnValue({ data: [store], isLoading: false, error: null });
    mockUseMemoryEntries.mockReturnValue({
      data: {
        store: "team",
        entries,
        invalid: [{ path: "/decisions/broken.md", error: "no header" }],
      },
      isLoading: false,
      error: null,
    });
    mockUseMemoryHistory.mockReturnValue({
      data: [
        {
          path: "/decisions/everruns-5528.md",
          action: "write",
          content: "---\nkind: decision\n---\nAllowlist",
          size_bytes: 30,
          changed_by: "session:abc",
          created_at: "2026-11-26T04:00:00Z",
        },
        {
          path: "/notes/old.md",
          action: "delete",
          size_bytes: 0,
          created_at: "2026-11-26T03:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });
  });

  it("orders open blocked items first and resolved items last", () => {
    const sections = groupMemoryEntries(entries).map((section) => section.key);
    expect(sections).toEqual(["blocked", "proposals", "decision", "resolved"]);
  });

  it("shows entries with their quotes, and files that do not parse", async () => {
    await renderPage();
    expect(screen.getByRole("heading", { name: /Housekeeping team/ })).toBeTruthy();
    expect(screen.getByText("Waiting on Mike to allowlist the package")).toBeTruthy();
    expect(screen.getAllByText("allow it", { selector: "blockquote" }).length).toBeGreaterThan(0);
    expect(screen.getByText("Proposals to review", { selector: "h2" })).toBeTruthy();
    expect(screen.getByText("/decisions/broken.md")).toBeTruthy();
    // A team memory is not renamed or archived from here.
    expect(screen.queryByRole("button", { name: /Edit/ })).toBeNull();
  });

  it("shows every kept change in History", async () => {
    await renderPage();
    fireEvent.click(screen.getByRole("tab", { name: "History" }));
    expect(screen.getByText("Written")).toBeTruthy();
    expect(screen.getByText("Deleted")).toBeTruthy();
    expect(screen.getByText("session:abc")).toBeTruthy();
  });

  it("reports a store the caller cannot see as missing", async () => {
    await renderPage("mem_ffffffffffffffffffffffffffffffff");
    expect(screen.getByText("Memory not found")).toBeTruthy();
  });
});
