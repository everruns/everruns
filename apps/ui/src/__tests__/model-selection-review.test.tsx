import { fireEvent, render, screen } from "@testing-library/react";
import { ModelSelectionDrawer } from "@/components/models/model-selection";
import type { ModelWithProvider } from "@/lib/api/types";
import type { ReactNode } from "react";

jest.mock("@/components/chat-mode/chat-mode-context", () => ({
  usePageChatMode: () => ({ open: true, pageContainer: { current: null } }),
}));
jest.mock("@/components/chat-mode/use-page-form-draft", () => ({
  usePageFormDraft: () => ({ adopted: true, blocked: false, notice: null }),
}));
jest.mock("@/components/ui/drawer", () => {
  const Section = ({ children }: { children: ReactNode }) => <div>{children}</div>;
  return {
    Drawer: Section,
    DrawerContent: Section,
    DrawerDescription: Section,
    DrawerFooter: Section,
    DrawerHeader: Section,
    DrawerTitle: Section,
  };
});
const model = (id: string, enabled: boolean, display_name = `Fixture ${id}`) =>
  ({
    id,
    model_id: id,
    display_name,
    provider_id: "provider_fixture",
    provider_name: "Fixture provider",
    provider_type: "openai",
    service: "chat",
    enabled,
    healthy: true,
    capabilities: ["chat"],
    created_at: "2026-10-26T00:00:00Z",
    updated_at: "2026-10-26T00:00:00Z",
  }) as ModelWithProvider;
function picker(models: ModelWithProvider[], apply: jest.Mock) {
  return (
    <ModelSelectionDrawer
      open
      onOpenChange={() => {}}
      title="Review fixture models"
      description="Select models"
      models={models}
      mode="edit"
      recommend={false}
      onApply={apply}
      applying={false}
    />
  );
}
it("keeps a dirty selection tied to the values displayed when editing began", () => {
  const apply = jest.fn();
  const { rerender } = render(picker([model("A", false)], apply));
  fireEvent.click(screen.getByRole("checkbox", { name: /Fixture A/ }));
  rerender(picker([model("A", true, "Foreign rename"), model("B", false)], apply));
  expect(screen.queryByText("Fixture B")).not.toBeInTheDocument();
  rerender(picker([model("A", true, "Second foreign rename"), model("B", true)], apply));
  expect(screen.queryByText("Fixture B")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Save/ }));
  expect([...apply.mock.calls[0][0]]).toEqual(["A"]);
  expect(apply.mock.calls[0][1]).toEqual([
    expect.objectContaining({ id: "A", enabled: false, display_name: "Fixture A" }),
  ]);
});
it("rebases a clean refresh and explicitly discarded work to the new catalog", () => {
  const apply = jest.fn();
  const { rerender } = render(picker([model("A", false)], apply));
  rerender(picker([model("A", true), model("B", false)], apply));
  expect(screen.getByRole("checkbox", { name: /Fixture A/ })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(screen.getByText("Fixture B")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("checkbox", { name: /Fixture A/ }));
  rerender(picker([model("A", true, "Fresh name"), model("B", false)], apply));
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByRole("checkbox", { name: /Fresh name/ })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: /Save/ }));
  expect(apply.mock.calls[0][1][0]).toMatchObject({ enabled: true, display_name: "Fresh name" });
});
