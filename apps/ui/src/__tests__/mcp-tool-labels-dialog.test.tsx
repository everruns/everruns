import { render, screen, within, fireEvent } from "@testing-library/react";
import { McpToolLabelsDialog, describeAnnotations } from "@/components/mcp/mcp-tool-labels-dialog";
import type { McpServer, McpServerTool } from "@/lib/api/types";

const mockUseMcpServerTools = jest.fn();
const mockSetLabel = jest.fn();
const mockSuggest = jest.fn();
let mockSuggestState: { isPending: boolean; error: Error | null } = {
  isPending: false,
  error: null,
};

jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServerTools: (id?: string) => mockUseMcpServerTools(id),
  useSetMcpToolLabel: () => ({
    mutate: mockSetLabel,
    isPending: false,
    error: null,
    variables: undefined,
  }),
  useSuggestMcpToolLabels: () => ({ mutate: mockSuggest, ...mockSuggestState }),
}));

const server = { id: "mcp_1", name: "docs", status: "active" } as McpServer;

const tools: McpServerTool[] = [
  {
    name: "publish",
    description: "Publish a page",
    annotations: null,
    label: null,
    suggested_label: "changes",
  },
  {
    name: "search",
    title: "Search docs",
    description: "Find pages",
    annotations: { readOnlyHint: true, openWorldHint: false },
    label: "read_only",
    suggested_label: null,
  },
];

function renderDialog(canEdit = true) {
  return render(
    <McpToolLabelsDialog server={server} canEdit={canEdit} open onOpenChange={jest.fn()} />,
  );
}

describe("McpToolLabelsDialog", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSuggestState = { isPending: false, error: null };
    mockUseMcpServerTools.mockReturnValue({ data: tools, isLoading: false, error: null });
  });

  it("explains approvals and lists each tool with what the server says", () => {
    renderDialog();

    expect(mockUseMcpServerTools).toHaveBeenCalledWith("mcp_1");
    expect(
      screen.getByText(
        "Choose which tools ask before they run. A read-only tool runs without asking. A tool that changes things asks first. By default every MCP tool asks.",
      ),
    ).toBeInTheDocument();
    const search = screen.getByTestId("tool-search");
    expect(within(search).getByText("Find pages")).toBeInTheDocument();
    expect(
      within(search).getByText("The server says it only reads, stays inside its own service."),
    ).toBeInTheDocument();
    expect(within(search).getByRole("radio", { name: "Read only" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    const publish = screen.getByTestId("tool-publish");
    expect(
      within(publish).getByText("The server says nothing about what this tool does."),
    ).toBeInTheDocument();
    expect(within(publish).getByRole("radio", { name: "Default" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("shows a suggestion without applying it, and uses it on request", () => {
    renderDialog();

    const publish = screen.getByTestId("tool-publish");
    expect(within(publish).getByText("Suggested: changes things")).toBeInTheDocument();
    expect(mockSetLabel).not.toHaveBeenCalled();

    fireEvent.click(within(publish).getByRole("button", { name: "Use suggestion" }));
    expect(mockSetLabel).toHaveBeenCalledWith({ toolName: "publish", label: "changes" });
  });

  it("sets and clears a label with the per-tool choice", () => {
    renderDialog();

    const publish = screen.getByTestId("tool-publish");
    fireEvent.click(within(publish).getByRole("radio", { name: "Read only" }));
    expect(mockSetLabel).toHaveBeenCalledWith({ toolName: "publish", label: "read_only" });

    const search = screen.getByTestId("tool-search");
    fireEvent.click(within(search).getByRole("radio", { name: "Default" }));
    expect(mockSetLabel).toHaveBeenCalledWith({ toolName: "search", label: null });
  });

  it("asks for suggestions and shows why it could not", () => {
    mockSuggestState = {
      isPending: false,
      error: new Error("Suggesting labels needs a decision service"),
    };
    renderDialog();

    fireEvent.click(screen.getByRole("button", { name: "Suggest labels" }));
    expect(mockSuggest).toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Suggesting labels needs a decision service",
    );
  });

  it("is read-only for people who cannot manage the server", () => {
    renderDialog(false);

    expect(screen.queryByRole("button", { name: "Suggest labels" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Use suggestion" })).not.toBeInTheDocument();
    for (const radio of screen.getAllByRole("radio")) {
      expect(radio).toBeDisabled();
    }
  });

  it("says when no tools are known yet", () => {
    mockUseMcpServerTools.mockReturnValue({ data: [], isLoading: false, error: null });
    renderDialog();

    expect(screen.getByText(/No tools are known for this server yet/)).toBeInTheDocument();
  });
});

describe("describeAnnotations", () => {
  it("puts the server's hints in plain words", () => {
    expect(describeAnnotations(null)).toEqual([]);
    expect(
      describeAnnotations({ readOnlyHint: false, destructiveHint: true, idempotentHint: true }),
    ).toEqual(["can change things", "can delete or overwrite", "safe to repeat"]);
  });
});
