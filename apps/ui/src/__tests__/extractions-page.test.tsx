import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import ExtractionsPage from "@/app/(main)/extractions/page";
import { api } from "@/lib/api/client";

jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_test" } }),
}));
jest.mock("@/providers/locale-provider", () => ({ useUiTranslations: () => (s: string) => s }));
jest.mock("@/lib/api/client", () => ({ api: { post: jest.fn() } }));
jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: ({ onChange, id }: { onChange: (value: string) => void; id: string }) => (
    <select id={id} onChange={(e) => onChange(e.target.value)}>
      <option value="">Select</option>
      <option value="model_test">Extraction model</option>
    </select>
  ),
}));
jest.mock("@/components/layout/page-layout", () => ({
  PageContainer: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  PageBreadcrumb: () => null,
  PageMasthead: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

beforeEach(() => jest.clearAllMocks());

it("submits the configured model and preserves nullable evidence in the result", async () => {
  (api.post as jest.Mock).mockResolvedValue({
    data: { answers: { total: { value: null, confidence: null, spans: null } } },
  });
  render(<ExtractionsPage />);
  fireEvent.change(screen.getByLabelText("Model"), { target: { value: "model_test" } });
  fireEvent.change(screen.getByLabelText("Source text"), {
    target: { value: "Invoice total unknown" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Extract" }));
  await waitFor(() =>
    expect(api.post).toHaveBeenCalledWith(
      "/v1/extractions",
      expect.objectContaining({
        model_id: "model_test",
        input: expect.objectContaining({ state: "Invoice total unknown" }),
      }),
    ),
  );
  expect(await screen.findByText(/"confidence": null/)).toBeInTheDocument();
});

it("does not send malformed field JSON to a provider", async () => {
  render(<ExtractionsPage />);
  fireEvent.change(screen.getByLabelText("Model"), { target: { value: "model_test" } });
  fireEvent.change(screen.getByLabelText("Source text"), { target: { value: "Invoice" } });
  fireEvent.change(screen.getByLabelText("Fields (JSON)"), { target: { value: "{" } });
  fireEvent.click(screen.getByRole("button", { name: "Extract" }));
  expect(await screen.findByRole("alert")).toBeInTheDocument();
  expect(api.post).not.toHaveBeenCalled();
});

it("does not expose provider error bodies", async () => {
  (api.post as jest.Mock).mockRejectedValue(new Error("secret upstream response"));
  render(<ExtractionsPage />);
  fireEvent.change(screen.getByLabelText("Model"), { target: { value: "model_test" } });
  fireEvent.change(screen.getByLabelText("Source text"), { target: { value: "Invoice" } });
  fireEvent.click(screen.getByRole("button", { name: "Extract" }));
  expect(await screen.findByRole("alert")).toBeInTheDocument();
  expect(screen.queryByText(/secret upstream/)).not.toBeInTheDocument();
});
