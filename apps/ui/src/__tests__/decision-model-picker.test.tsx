import { render, screen } from "@testing-library/react";
import { ModelPicker, DecisionModelPicker } from "@/components/models/model-picker";
import type { ModelWithProvider } from "@/lib/api/types";
const mockModels = [
  { id: "chat", display_name: "Chat", service: "chat", enabled: true, healthy: true },
  {
    id: "embedding",
    display_name: "Embedding",
    service: "embeddings",
    enabled: true,
    healthy: true,
  },
  {
    id: "jev",
    display_name: "Jev",
    service: "decisions",
    enabled: true,
    healthy: true,
    profile: { decisions: { calibrated: true, primitives: ["noul", "choice", "score"] } },
  },
  {
    id: "broken",
    display_name: "Broken Jev",
    service: "decisions",
    enabled: true,
    healthy: false,
    profile: { decisions: { calibrated: true, primitives: ["noul"] } },
  },
  {
    id: "preview",
    display_name: "Preview",
    service: "decisions",
    enabled: true,
    healthy: true,
    profile: { decisions: { calibrated: false, primitives: ["choice"] } },
  },
].map((m) => ({
  ...m,
  provider_name: "Account",
  provider_id: "provider",
  is_favorite: false,
})) as ModelWithProvider[];
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({ data: mockModels }),
  useUpdateModel: () => ({ mutate: jest.fn() }),
}));
jest.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: jest.fn() }),
}));
jest.mock("@/components/models/model-icon", () => ({ ModelIcon: () => null }));
jest.mock("@/components/ui/select", () => ({
  Select: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectTrigger: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectValue: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectItem: ({ children, value }: { children: React.ReactNode; value: string }) => (
    <div data-testid={value}>{children}</div>
  ),
}));
test("chat selectors exclude decision and embedding models", () => {
  render(<ModelPicker value="" onChange={jest.fn()} />);
  expect(screen.getByTestId("chat")).toBeInTheDocument();
  expect(screen.queryByTestId("jev")).not.toBeInTheDocument();
  expect(screen.queryByTestId("embedding")).not.toBeInTheDocument();
});
test("decisions require calibrated supported primitives and healthy accounts", () => {
  render(
    <DecisionModelPicker
      requiredPrimitives={["noul", "choice", "score"]}
      value=""
      onChange={jest.fn()}
    />,
  );
  expect(screen.getByTestId("jev")).toBeInTheDocument();
  for (const id of ["chat", "embedding", "broken", "preview"])
    expect(screen.queryByTestId(id)).not.toBeInTheDocument();
});
test("a saved unavailable binding remains visible with repair guidance", () => {
  render(<DecisionModelPicker value="broken" onChange={jest.fn()} />);
  expect(screen.getByText("Broken Jev (Account)")).toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent("repair");
});
