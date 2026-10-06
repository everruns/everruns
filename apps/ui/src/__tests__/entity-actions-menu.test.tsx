import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import {
  buildEntityMenuGroups,
  EntityActionsMenu,
  type EntityActionsMenuProps,
} from "@/components/entity-actions/entity-actions-menu";
import { describeChangeAction, secretNames } from "@/components/entity-actions/history-sheet";
import { ApiError } from "@/lib/api/client";
import type { EntityChange } from "@/lib/api/types";

const mockReplace = jest.fn();
let mockSearch = "";

jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mockReplace, push: jest.fn() }),
  usePathname: () => "/agents/agent_1",
  useSearchParams: () => new URLSearchParams(mockSearch),
}));

jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ children }: { children: string }) => <div>{children}</div>,
}));

jest.mock("@/hooks/use-members", () => ({
  useMembers: () => ({ data: [{ user_id: "user-1", name: "Ada", email: "ada@example.com" }] }),
}));

const mockRestore = jest.fn();
const mockSaveNotes = jest.fn();
let mockHistory: EntityChange[] = [];
let mockDiff: unknown[] = [];
let mockSnapshot: unknown = { name: "a", $secrets: { api_key: { set: true, fingerprint: "f" } } };
let mockNotes = { content: "", revision: 0 };
let mockSaveError: unknown = null;

jest.mock("@/hooks/use-change-history", () => ({
  useEntityHistory: () => ({
    data: { pages: [{ entries: mockHistory }] },
    isLoading: false,
    error: null,
    hasNextPage: false,
  }),
  useEntityRevision: (_ref: string, revision: number | null) => ({
    data: revision === null ? undefined : { revision, snapshot: mockSnapshot },
    isLoading: false,
    error: null,
  }),
  useEntityRevisionDiff: () => ({ data: mockDiff, isLoading: false, error: null }),
  useRestoreEntityRevision: () => ({
    mutateAsync: mockRestore,
    isPending: false,
    error: null,
    reset: jest.fn(),
  }),
  useManagerContext: () => ({
    data: mockNotes,
    isLoading: false,
    error: null,
    refetch: jest.fn(),
  }),
  useSetManagerContext: () => ({
    mutateAsync: mockSaveNotes,
    isPending: false,
    error: mockSaveError,
    reset: jest.fn(),
  }),
}));

function change(overrides: Partial<EntityChange> = {}): EntityChange {
  return {
    id: "c1",
    entity_kind: "agent",
    entity_ref: "agent_1",
    action: "updated",
    command: "update_agent",
    changed_fields: ["system_prompt"],
    actor_kind: "user",
    actor_user_id: "user-1",
    surface: "api",
    created_at: "2026-10-01T10:00:00Z",
    revision: 2,
    reason: "make it kid friendly",
    ...overrides,
  };
}

const baseProps: EntityActionsMenuProps = {
  entityRef: "agent_1",
  kind: "agent",
  entityName: "Jokes Agent",
  permissions: { manage: true },
};

beforeEach(() => {
  jest.clearAllMocks();
  mockSearch = "";
  mockHistory = [];
  mockDiff = [];
  mockNotes = { content: "", revision: 0 };
  mockSaveError = null;
  mockSnapshot = { name: "a", $secrets: { api_key: { set: true, fingerprint: "f" } } };
});

describe("buildEntityMenuGroups", () => {
  const noop = () => undefined;

  it("orders entity actions, record, then lifecycle with delete last", () => {
    const groups = buildEntityMenuGroups({
      ...baseProps,
      actions: [{ id: "copy", label: "Copy", onSelect: noop }],
      archive: { onSelect: noop },
      delete: { onSelect: noop },
    });
    expect(groups.map((g) => g.id)).toEqual(["entity", "record", "lifecycle"]);
    expect(groups[1].items.map((i) => i.id)).toEqual(["history", "notes"]);
    const lifecycle = groups[2].items;
    expect(lifecycle.map((i) => i.id)).toEqual(["archive", "delete"]);
    expect(lifecycle.at(-1)).toMatchObject({ id: "delete", destructive: true });
  });

  it("hides manager notes from viewers who do not manage the entity", () => {
    const groups = buildEntityMenuGroups({ ...baseProps, permissions: { manage: false } });
    expect(groups.map((g) => g.id)).toEqual(["record"]);
    expect(groups[0].items.map((i) => i.id)).toEqual(["history"]);
  });

  it("omits empty groups", () => {
    expect(
      buildEntityMenuGroups({
        ...baseProps,
        permissions: { manage: false, history: false },
        delete: { onSelect: noop },
      }).map((g) => g.id),
    ).toEqual(["lifecycle"]);
    expect(
      buildEntityMenuGroups({ ...baseProps, permissions: { manage: false, history: false } }),
    ).toEqual([]);
  });
});

describe("EntityActionsMenu", () => {
  it("labels the trigger with the entity name and opens History in the address", async () => {
    const onDelete = jest.fn();
    render(
      <EntityActionsMenu
        {...baseProps}
        actions={[{ id: "copy", label: "Copy", onSelect: jest.fn() }]}
        archive={{ onSelect: jest.fn() }}
        delete={{ onSelect: onDelete }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "More actions for Jokes Agent" }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      "Copy",
      "History",
      "Manager notes",
      "Archive agent",
      "Delete agent",
    ]);
    expect(items.at(-1)).toHaveAttribute("data-variant", "destructive");
    expect(screen.getAllByRole("separator")).toHaveLength(2);

    fireEvent.click(screen.getByRole("menuitem", { name: "History" }));
    expect(mockReplace).toHaveBeenCalledWith("/agents/agent_1?sheet=history", { scroll: false });
  });

  it("does not offer Manager notes to a reader", async () => {
    render(<EntityActionsMenu {...baseProps} permissions={{ manage: false }} />);
    fireEvent.click(screen.getByRole("button", { name: "More actions for Jokes Agent" }));
    expect(await screen.findByRole("menuitem", { name: "History" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Manager notes" })).not.toBeInTheDocument();
    expect(screen.queryByRole("separator")).not.toBeInTheDocument();
  });
});

describe("HistorySheet", () => {
  it("shows the empty state", async () => {
    mockSearch = "sheet=history";
    render(<EntityActionsMenu {...baseProps} />);
    expect(await screen.findByText("No changes recorded yet.")).toBeInTheDocument();
  });

  it("lists entries and restores a point only with a reason", async () => {
    mockSearch = "sheet=history";
    mockHistory = [
      change({ id: "c2", action: "restored", restored_from_revision: 1, revision: 3 }),
      change(),
    ];
    mockDiff = [
      { field: "system_prompt", from: "Tell a joke.", to: "Tell a kid-friendly joke." },
      { field: "$secrets.api_key", from: { set: true, fingerprint: "a" }, to: null },
    ];
    mockRestore.mockResolvedValue({ restored_revision: 2, entity: {}, warnings: ["kept api_key"] });
    render(<EntityActionsMenu {...baseProps} />);

    expect(await screen.findByText("Restored revision 1")).toBeInTheDocument();
    expect(screen.getAllByText(/make it kid friendly/)).toHaveLength(2);
    expect(screen.getAllByText("Ada").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: /^Updated/ }));
    expect(await screen.findByText("Tell a joke.")).toBeInTheDocument();
    // Secrets show as changed or unchanged, never a value or fingerprint.
    const secrets = screen.getByRole("list", { name: "Secrets" });
    expect(within(secrets).getByText("api_key")).toBeInTheDocument();
    expect(within(secrets).getByText("changed")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Restore this point" }));
    const dialog = await screen.findByRole("dialog", { name: "Restore revision 2?" });
    expect(within(dialog).getByText(/will not be restored/)).toHaveTextContent("api_key");
    const submit = within(dialog).getByRole("button", { name: "Restore" });
    expect(submit).toBeDisabled();
    fireEvent.change(within(dialog).getByLabelText(/Reason for this change/), {
      target: { value: "the old prompt was better" },
    });
    expect(submit).toBeEnabled();
    fireEvent.click(submit);
    await waitFor(() =>
      expect(mockRestore).toHaveBeenCalledWith({
        revision: 2,
        reason: "the old prompt was better",
      }),
    );
    expect(await screen.findByText("kept api_key")).toBeInTheDocument();
  });

  it("hides restore from readers", async () => {
    mockSearch = "sheet=history";
    mockHistory = [change()];
    mockDiff = [{ field: "name", from: "a", to: "b" }];
    render(<EntityActionsMenu {...baseProps} permissions={{ manage: false }} />);
    fireEvent.click(await screen.findByRole("button", { name: /^Updated/ }));
    expect(await screen.findByText("Compared with now")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restore this point" })).not.toBeInTheDocument();
  });
});

describe("ManagerNotesSheet", () => {
  it("explains notes in the empty state", async () => {
    mockSearch = "sheet=notes";
    render(<EntityActionsMenu {...baseProps} />);
    expect(await screen.findByText("No manager notes yet")).toBeInTheDocument();
    expect(screen.getByText(/The agent itself never sees them/)).toBeInTheDocument();
  });

  it("saves with the revision it read and an optional reason", async () => {
    mockSearch = "sheet=notes";
    mockNotes = { content: "Keep it short.", revision: 4 };
    mockSaveNotes.mockResolvedValue({});
    render(<EntityActionsMenu {...baseProps} />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Manager notes" }), {
      target: { value: "Keep it short. No puns." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save notes" }));
    await waitFor(() =>
      expect(mockSaveNotes).toHaveBeenCalledWith({
        content: "Keep it short. No puns.",
        expectedRevision: 4,
        reason: "",
      }),
    );
  });

  it("shows a conflict when someone else saved first", async () => {
    mockSearch = "sheet=notes";
    mockNotes = { content: "Keep it short.", revision: 4 };
    mockSaveError = new ApiError(409, "Conflict", "stale", "manager_context_changed");
    render(<EntityActionsMenu {...baseProps} />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    expect(screen.getByText("Someone else changed these notes")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save notes" })).toBeDisabled();
  });

  it("is not rendered for readers even when addressed", () => {
    mockSearch = "sheet=notes";
    render(<EntityActionsMenu {...baseProps} permissions={{ manage: false }} />);
    expect(screen.queryByText("Manager notes")).not.toBeInTheDocument();
  });
});

describe("history helpers", () => {
  it("describes a restore with the revision it brought back", () => {
    expect(describeChangeAction(change({ action: "restored", restored_from_revision: 7 }))).toBe(
      "Restored revision 7",
    );
    expect(describeChangeAction(change({ action: "context_set" }))).toBe("Context set");
  });

  it("names changed secrets from field or whole-map diffs", () => {
    expect(
      secretNames([
        { field: "name", from: "a", to: "b" },
        { field: "$secrets.token", from: null, to: { set: true } },
        {
          field: "$secrets",
          from: { a: { set: true, fingerprint: "1" }, b: { set: true, fingerprint: "2" } },
          to: { a: { set: true, fingerprint: "1" }, b: { set: true, fingerprint: "3" } },
        },
      ]),
    ).toEqual(["b", "token"]);
  });
});
