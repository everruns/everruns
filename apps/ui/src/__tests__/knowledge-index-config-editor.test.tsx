import { render, screen, fireEvent, within } from "@testing-library/react";
import {
  KnowledgeIndexConfigEditor,
  readIndexes,
  readTopK,
  validateTopK,
} from "@/components/agents/knowledge-index-config-editor";

const INDEX_A = "kidx_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const INDEX_B = "kidx_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const INDEX_C = "kidx_cccccccccccccccccccccccccccccccc";

jest.mock("@/hooks/use-knowledge-indexes", () => ({
  useKnowledgeIndexes: jest.fn(),
}));

const { useKnowledgeIndexes } = jest.requireMock("@/hooks/use-knowledge-indexes") as {
  useKnowledgeIndexes: jest.Mock;
};

beforeEach(() => {
  useKnowledgeIndexes.mockReturnValue({
    data: [
      { id: INDEX_A, name: "Product Docs", status: "active" },
      { id: INDEX_B, name: "API Reference", status: "active" },
    ],
    isLoading: false,
    error: null,
  });
});

describe("readIndexes", () => {
  it("reads and de-duplicates index ids", () => {
    expect(readIndexes({ indexes: [INDEX_A, INDEX_A, INDEX_B] })).toEqual([INDEX_A, INDEX_B]);
  });

  it("treats null/non-object config as empty", () => {
    expect(readIndexes(null)).toEqual([]);
    expect(readIndexes({})).toEqual([]);
  });
});

describe("readTopK / validateTopK", () => {
  it("reads numeric top_k", () => {
    expect(readTopK({ top_k: 5 })).toBe(5);
    expect(readTopK({})).toBeUndefined();
  });

  it("accepts in-range integers and rejects out-of-range", () => {
    expect(validateTopK(undefined)).toBeNull();
    expect(validateTopK(1)).toBeNull();
    expect(validateTopK(50)).toBeNull();
    expect(validateTopK(0)).not.toBeNull();
    expect(validateTopK(51)).not.toBeNull();
    expect(validateTopK(2.5)).not.toBeNull();
  });
});

function clickRowCheckbox(name: string) {
  const row = screen.getByText(name).closest('[data-testid="knowledge-index-row"]');
  if (!row) throw new Error(`row for ${name} not found`);
  fireEvent.click(within(row as HTMLElement).getByRole("checkbox"));
}

describe("KnowledgeIndexConfigEditor", () => {
  it("renders empty state when no indexes and no selection", () => {
    useKnowledgeIndexes.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<KnowledgeIndexConfigEditor config={{}} onChange={jest.fn()} />);
    expect(screen.getByText(/no active knowledge indexes/i)).toBeInTheDocument();
  });

  it("renders a row per active index", () => {
    render(<KnowledgeIndexConfigEditor config={{}} onChange={jest.fn()} />);
    expect(screen.getAllByTestId("knowledge-index-row")).toHaveLength(2);
    expect(screen.getByText("Product Docs")).toBeInTheDocument();
    expect(screen.getByText("API Reference")).toBeInTheDocument();
  });

  it("treats null config as empty selection", () => {
    render(<KnowledgeIndexConfigEditor config={null} onChange={jest.fn()} />);
    const checkboxes = screen.getAllByRole("checkbox");
    checkboxes.forEach((checkbox) => expect(checkbox).not.toBeChecked());
  });

  it("emits config with the selected index when a row is checked", () => {
    const onChange = jest.fn();
    render(<KnowledgeIndexConfigEditor config={{}} onChange={onChange} />);
    clickRowCheckbox("Product Docs");
    expect(onChange).toHaveBeenCalledWith({ indexes: [INDEX_A] });
  });

  it("removes a selected index when unchecked, clearing to empty config", () => {
    const onChange = jest.fn();
    render(<KnowledgeIndexConfigEditor config={{ indexes: [INDEX_A] }} onChange={onChange} />);
    clickRowCheckbox("Product Docs");
    expect(onChange).toHaveBeenCalledWith({});
  });

  it("emits top_k when the number input changes", () => {
    const onChange = jest.fn();
    render(<KnowledgeIndexConfigEditor config={{ indexes: [INDEX_A] }} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/top k/i), { target: { value: "7" } });
    expect(onChange).toHaveBeenCalledWith({ indexes: [INDEX_A], top_k: 7 });
  });

  it("clears top_k when the input is emptied", () => {
    const onChange = jest.fn();
    render(
      <KnowledgeIndexConfigEditor config={{ indexes: [INDEX_A], top_k: 7 }} onChange={onChange} />,
    );
    fireEvent.change(screen.getByLabelText(/top k/i), { target: { value: "" } });
    expect(onChange).toHaveBeenCalledWith({ indexes: [INDEX_A] });
  });

  it("shows an error for an out-of-range top_k", () => {
    render(<KnowledgeIndexConfigEditor config={{ top_k: 99 }} onChange={jest.fn()} />);
    expect(screen.getByText(/top_k must be between 1 and 50/i)).toBeInTheDocument();
  });

  it("surfaces selected indexes that are no longer active", () => {
    render(<KnowledgeIndexConfigEditor config={{ indexes: [INDEX_C] }} onChange={jest.fn()} />);
    expect(screen.getByText(/unavailable or archived/i)).toBeInTheDocument();
  });

  it("does not flag unknown indexes while the list is loading", () => {
    useKnowledgeIndexes.mockReturnValue({ data: undefined, isLoading: true, error: null });
    render(<KnowledgeIndexConfigEditor config={{ indexes: [INDEX_C] }} onChange={jest.fn()} />);
    expect(screen.queryByText(/unavailable or archived/i)).not.toBeInTheDocument();
  });
});
