import { render, screen, fireEvent, within } from "@testing-library/react";
import { KnowledgeConfigEditor } from "@/components/agents/knowledge-config-editor";

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_test" } }),
}));
jest.mock("@/lib/api/knowledge", () => ({ listKnowledge: jest.fn() }));
jest.mock("@tanstack/react-query", () => ({ useInfiniteQuery: jest.fn() }));
const { useInfiniteQuery } = jest.requireMock("@tanstack/react-query");
const docs = { id: "kn_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", name: "everruns-docs", indexing: "off" };
const support = { id: "kn_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", name: "Support", indexing: "lexical" };
beforeEach(() => {
  useInfiniteQuery.mockReturnValue({ data: { pages: [{ data: [docs, support] }] } });
});
function checkbox(name: string) {
  return within(screen.getByText(name).closest("label")!).getByRole("checkbox");
}
it("attaches unindexed files and preserves automatic disclosure settings", () => {
  const onChange = jest.fn();
  render(<KnowledgeConfigEditor config={{ automatic: true, limit: 4 }} onChange={onChange} />);
  expect(screen.getByText("Files only")).toBeInTheDocument();
  fireEvent.click(checkbox("everruns-docs"));
  expect(onChange).toHaveBeenCalledWith({ automatic: true, limit: 4, collections: [docs.id] });
});
it("recognizes the built-in alias and removes both equivalent selections", () => {
  const onChange = jest.fn();
  render(
    <KnowledgeConfigEditor
      config={{ collections: ["everruns-docs", docs.id] }}
      onChange={onChange}
    />,
  );
  expect(checkbox("everruns-docs")).toBeChecked();
  fireEvent.click(checkbox("everruns-docs"));
  expect(onChange).toHaveBeenCalledWith({ collections: [] });
});
it("bounds attachment count while allowing removal", () => {
  const collections = [support.id, ...Array.from({ length: 31 }, (_, i) => `kn_${i}`)];
  render(<KnowledgeConfigEditor config={{ collections }} onChange={jest.fn()} />);
  expect(checkbox("everruns-docs")).toBeDisabled();
  expect(checkbox("Support")).not.toBeDisabled();
});
it("paginates discovery without changing selections", () => {
  const fetchNextPage = jest.fn();
  useInfiniteQuery.mockReturnValue({
    data: { pages: [{ data: [docs] }] },
    hasNextPage: true,
    fetchNextPage,
  });
  render(<KnowledgeConfigEditor config={{}} onChange={jest.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Load more collections" }));
  expect(fetchNextPage).toHaveBeenCalledTimes(1);
});
it("disables binding and disclosure controls when editing is forbidden", () => {
  render(<KnowledgeConfigEditor config={{}} onChange={jest.fn()} disabled />);
  screen.getAllByRole("checkbox").forEach((element) => expect(element).toBeDisabled());
  expect(screen.getByLabelText("Result limit")).toBeDisabled();
});
