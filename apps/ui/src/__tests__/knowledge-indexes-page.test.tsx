import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import * as mockReact from "react";
import KnowledgeIndexesPage from "@/app/(main)/knowledge-indexes/page";
import type { KnowledgeIndex } from "@/lib/api/types";

const mockUseKnowledgeIndexes = jest.fn();
const mockUseCreateKnowledgeIndex = jest.fn();
const mockUseUpdateKnowledgeIndex = jest.fn();
const mockUseSyncKnowledgeIndex = jest.fn();
const mockUseArchiveKnowledgeIndex = jest.fn();
const mockUseUserConnections = jest.fn();
const mockUseModels = jest.fn();

jest.mock("@/hooks", () => ({
  useKnowledgeIndexes: (...args: unknown[]) => mockUseKnowledgeIndexes(...args),
  useCreateKnowledgeIndex: () => mockUseCreateKnowledgeIndex(),
  useUpdateKnowledgeIndex: () => mockUseUpdateKnowledgeIndex(),
  useSyncKnowledgeIndex: () => mockUseSyncKnowledgeIndex(),
  useArchiveKnowledgeIndex: () => mockUseArchiveKnowledgeIndex(),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useUserConnections: () => mockUseUserConnections(),
}));

jest.mock("@/hooks/use-providers", () => ({
  useModels: () => mockUseModels(),
}));

jest.mock("@/components/ui/select", () => {
  function collectOptions(node: mockReact.ReactNode): mockReact.ReactNode[] {
    const options: mockReact.ReactNode[] = [];
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child)) return;
      if ((child.type as { isSelectItem?: boolean }).isSelectItem) {
        const props = child.props as {
          value: string;
          children: mockReact.ReactNode;
          disabled?: boolean;
        };
        options.push(
          <option key={props.value} value={props.value} disabled={props.disabled}>
            {props.children}
          </option>,
        );
        return;
      }
      options.push(...collectOptions((child.props as { children?: mockReact.ReactNode }).children));
    });
    return options;
  }

  // Find the SelectTrigger child so its id/aria-label can be lifted onto the
  // native <select> the mock renders (mirrors how the real trigger is the
  // labelable control).
  function findTrigger(node: mockReact.ReactNode): {
    id?: string;
    ariaLabel?: string;
    ariaInvalid?: boolean;
    ariaDescribedBy?: string;
  } {
    let found: {
      id?: string;
      ariaLabel?: string;
      ariaInvalid?: boolean;
      ariaDescribedBy?: string;
    } = {};
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child) || found.id || found.ariaLabel) return;
      if ((child.type as { isSelectTrigger?: boolean }).isSelectTrigger) {
        const props = child.props as {
          id?: string;
          "aria-label"?: string;
          "aria-invalid"?: boolean;
          "aria-describedby"?: string;
        };
        found = {
          id: props.id,
          ariaLabel: props["aria-label"],
          ariaInvalid: props["aria-invalid"],
          ariaDescribedBy: props["aria-describedby"],
        };
        return;
      }
      const nested = findTrigger((child.props as { children?: mockReact.ReactNode }).children);
      if (nested.id || nested.ariaLabel) found = nested;
    });
    return found;
  }

  function Select({
    value,
    onValueChange,
    children,
    "aria-label": ariaLabel,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: mockReact.ReactNode;
    "aria-label"?: string;
  }) {
    const trigger = findTrigger(children);
    return (
      <select
        id={trigger.id}
        aria-label={trigger.ariaLabel ?? ariaLabel ?? "Source"}
        aria-invalid={trigger.ariaInvalid}
        aria-describedby={trigger.ariaDescribedBy}
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        {collectOptions(children)}
      </select>
    );
  }

  function SelectItem(_: { value: string; children: mockReact.ReactNode }) {
    return null;
  }
  SelectItem.isSelectItem = true;

  function SelectTrigger(_: {
    children: mockReact.ReactNode;
    id?: string;
    "aria-label"?: string;
    "aria-invalid"?: boolean;
    "aria-describedby"?: string;
  }) {
    return null;
  }
  SelectTrigger.isSelectTrigger = true;

  return {
    Select,
    SelectContent: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectTrigger,
    SelectValue: () => null,
  };
});

const indexes: KnowledgeIndex[] = [
  {
    id: "kidx_019dfb261a407c6085dcdd602402c3f7",
    name: "Product Docs",
    description: "Synced product documentation",
    source_type: "github",
    source_config: { provider: "github", repository: "everruns/everruns", branch: "main" },
    embedding_model_id: "model_001",
    vector_dim: 1536,
    document_count: 1,
    status: "active",
    sync_status: "synced",
    last_synced_at: "2026-05-06T03:37:51.552849Z",
    last_sync_error: null,
    created_at: "2026-05-06T02:37:51.552849Z",
    updated_at: "2026-05-06T02:37:51.552849Z",
    archived_at: null,
    deleted_at: null,
  },
];

describe("KnowledgeIndexesPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseKnowledgeIndexes.mockReturnValue({ data: indexes, isLoading: false, error: null });
    mockUseCreateKnowledgeIndex.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseUpdateKnowledgeIndex.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseSyncKnowledgeIndex.mockReturnValue({ mutate: jest.fn(), isPending: false });
    mockUseArchiveKnowledgeIndex.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseUserConnections.mockReturnValue({
      data: [{ provider: "github", connected_at: "2026-05-01T00:00:00Z" }],
      isLoading: false,
      error: null,
    });
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_001",
          display_name: "text-embedding-3-small",
          provider_name: "OpenAI",
          provider_id: "provider_001",
          provider_type: "openai_completions",
          healthy: true,
          enabled: true,
          capabilities: ["embeddings"],
        },
      ],
      isLoading: false,
    });
  });

  it("renders indexes with status, source, and an open link", () => {
    render(<KnowledgeIndexesPage />);
    expect(
      screen.getByRole("heading", { level: 1, name: "Knowledge Indexes" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Product Docs")).toBeInTheDocument();
    expect(screen.getByText("Synced product documentation")).toBeInTheDocument();
    expect(screen.getAllByText("synced").length).toBeGreaterThan(0);
    expect(screen.getByRole("link", { name: /Open/i })).toHaveAttribute(
      "href",
      "/knowledge-indexes/kidx_019dfb261a407c6085dcdd602402c3f7",
    );
  });

  it("passes search text into the query", () => {
    render(<KnowledgeIndexesPage />);
    fireEvent.change(screen.getByLabelText("Search knowledge indexes"), {
      target: { value: "docs" },
    });
    expect(mockUseKnowledgeIndexes).toHaveBeenLastCalledWith({
      includeArchived: true,
      search: "docs",
    });
  });

  it("creates an index from the dialog", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateKnowledgeIndex.mockReturnValue({ mutateAsync, isPending: false });
    render(<KnowledgeIndexesPage />);

    fireEvent.click(screen.getByRole("button", { name: "New index" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "API Docs" } });
    fireEvent.change(within(dialog).getByLabelText("Embedding model"), {
      target: { value: "model_001" },
    });
    fireEvent.change(within(dialog).getByLabelText("Repository"), {
      target: { value: "everruns/everruns" },
    });
    fireEvent.change(within(dialog).getByLabelText("Branch"), { target: { value: "main" } });
    fireEvent.change(within(dialog).getByLabelText("Root Folder"), {
      target: { value: "docs" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Index" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "API Docs",
        source_type: "github",
        source_config: {
          provider: "github",
          repository: "everruns/everruns",
          branch: "main",
          root_folder: "docs",
        },
        embedding_model_id: "model_001",
      }),
    );
  });

  it("requires an embedding model before submit", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateKnowledgeIndex.mockReturnValue({ mutateAsync, isPending: false });
    render(<KnowledgeIndexesPage />);

    fireEvent.click(screen.getByRole("button", { name: "New index" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "No Model" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Index" }));

    const nameInput = within(dialog).getByLabelText("Name");
    const embeddingModelPicker = within(dialog).getByRole("combobox", {
      name: "Embedding model",
    });
    const error = within(dialog).getByText(/embedding model is required/i);

    expect(nameInput).not.toHaveAttribute("aria-invalid", "true");
    expect(embeddingModelPicker).toHaveAttribute("aria-invalid", "true");
    expect(embeddingModelPicker).toHaveAttribute("aria-describedby", error.id);
    expect(mutateAsync).not.toHaveBeenCalled();
  });

  it("associates multiple validation errors with their fields", () => {
    render(<KnowledgeIndexesPage />);

    fireEvent.click(screen.getByRole("button", { name: "New index" }));
    const dialog = screen.getByRole("dialog");
    const submitButton = within(dialog).getByRole("button", { name: "Create Index" });
    fireEvent.submit(submitButton.closest("form")!);

    const nameInput = within(dialog).getByLabelText("Name");
    const embeddingModelPicker = within(dialog).getByRole("combobox", {
      name: "Embedding model",
    });
    const nameError = within(dialog).getByText(/^name is required$/i);
    const modelError = within(dialog).getByText(/^embedding model is required$/i);

    expect(nameInput).toHaveAttribute("aria-invalid", "true");
    expect(nameInput).toHaveAttribute("aria-describedby", nameError.id);
    expect(embeddingModelPicker).toHaveAttribute("aria-invalid", "true");
    expect(embeddingModelPicker).toHaveAttribute("aria-describedby", modelError.id);
  });

  it("does not offer chat models when no embedding models are configured", () => {
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_chat",
          display_name: "GPT-5.6",
          provider_name: "OpenAI",
          enabled: true,
          capabilities: ["chat", "tools"],
        },
      ],
      isLoading: false,
    });

    render(<KnowledgeIndexesPage />);
    fireEvent.click(screen.getByRole("button", { name: "New index" }));

    const picker = within(screen.getByRole("dialog")).getByLabelText("Embedding model");
    expect(within(picker).queryByRole("option", { name: "GPT-5.6 (OpenAI)" })).toBeNull();
    expect(
      within(picker).getByRole("option", { name: "No embedding models available" }),
    ).toBeDisabled();
    expect(
      within(screen.getByRole("dialog")).getByRole("link", {
        name: "Configure an embedding model",
      }),
    ).toHaveAttribute("href", "/models");
  });

  it("archives an index after confirmation", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseArchiveKnowledgeIndex.mockReturnValue({ mutateAsync, isPending: false });
    render(<KnowledgeIndexesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Archive Product Docs?")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith("kidx_019dfb261a407c6085dcdd602402c3f7"),
    );
  });

  it("renders the empty state", () => {
    mockUseKnowledgeIndexes.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<KnowledgeIndexesPage />);
    expect(screen.getByText(/No knowledge indexes/)).toBeInTheDocument();
  });

  it.each([
    [
      "not synced yet",
      { sync_status: "idle", last_synced_at: null, document_count: 0 },
      "Not synced yet",
      "Start initial sync",
    ],
    ["queued", { sync_status: "pending" }, "Sync queued", null],
    ["syncing", { sync_status: "syncing" }, "Syncing", null],
    ["synced empty", { document_count: 0 }, "Synced — no documents", "Sync again"],
    ["stale", { updated_at: "2026-05-06T04:37:51.552849Z" }, "Sync out of date", "Sync changes"],
    [
      "failed",
      { sync_status: "failed", last_sync_error: "Repository access was denied" },
      "Sync failed",
      "Retry sync",
    ],
  ])("shows the %s diagnostic and action", (_name, overrides, label, action) => {
    mockUseKnowledgeIndexes.mockReturnValue({
      data: [{ ...indexes[0], ...overrides }],
      isLoading: false,
      error: null,
    });
    render(<KnowledgeIndexesPage />);

    expect(screen.getByRole("status", { name: `Index status: ${label}` })).toBeInTheDocument();
    if (action) expect(screen.getByRole("button", { name: action })).toBeInTheDocument();
  });

  it("flags a generation-only model and directs the user to configure embeddings", () => {
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_001",
          display_name: "Claude Opus 4.7 (1M)",
          provider_name: "Anthropic",
          provider_id: "provider_anthropic",
          provider_type: "anthropic",
          enabled: true,
          healthy: true,
          capabilities: ["text"],
        },
      ],
      isLoading: false,
      error: null,
    });
    render(<KnowledgeIndexesPage />);

    expect(
      screen.getByRole("status", { name: "Index status: Embedding model invalid" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/does not support embeddings/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configure embedding model" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sync" })).not.toBeInTheDocument();
  });

  it("links to provider setup when the embedding provider is unavailable", () => {
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_001",
          display_name: "text-embedding-3-small",
          provider_name: "OpenAI",
          provider_id: "provider_001",
          provider_type: "openai_completions",
          enabled: true,
          healthy: false,
          capabilities: ["embeddings"],
        },
      ],
      isLoading: false,
      error: null,
    });
    render(<KnowledgeIndexesPage />);

    expect(
      screen.getByRole("status", { name: "Index status: Provider setup needed" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Configure provider" })).toHaveAttribute(
      "href",
      "/settings/providers/provider_001",
    );
  });

  it("updates the diagnostic when polled index data changes", () => {
    mockUseKnowledgeIndexes.mockReturnValue({
      data: [{ ...indexes[0], sync_status: "pending" }],
      isLoading: false,
      error: null,
    });
    const view = render(<KnowledgeIndexesPage />);
    expect(screen.getByRole("status", { name: "Index status: Sync queued" })).toBeInTheDocument();

    mockUseKnowledgeIndexes.mockReturnValue({ data: indexes, isLoading: false, error: null });
    view.rerender(<KnowledgeIndexesPage />);
    expect(screen.getByRole("status", { name: "Index status: Up to date" })).toBeInTheDocument();
  });
});
