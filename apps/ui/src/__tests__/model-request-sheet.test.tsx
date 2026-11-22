import { fireEvent, render, screen, within } from "@testing-library/react";
import { RequestSheet } from "@/components/session/trace/request-sheet";
import type { TraceRequestPage } from "@/lib/api/types";

const imageId = "img_00000000000000000000000000000001";
const fileId = "file_00000000000000000000000000000002";
const mockPage: TraceRequestPage = {
  message_count: 1,
  new_from: 0,
  has_native_request: true,
  model: "fixture-model",
  tools: [{ name: "read_file", description: "Read a workspace file" }],
  messages: [
    {
      index: 0,
      role: "user",
      preview: "Read the attached report.",
      size_bytes: 640,
      content: {
        content: [
          { type: "text", text: "Read the attached report." },
          { type: "image_file", image_id: imageId, filename: "chart.png" },
          { type: "file", file_id: fileId, filename: "report.pdf" },
          { type: "image", url: "https://external.example/track.png" },
        ],
      },
    },
  ],
};

jest.mock("@/hooks/use-session-trace", () => ({
  useTraceRequest: () => ({ data: { pages: [mockPage] } }),
  useTraceRequestMessage: () => ({}),
}));

function renderSheet() {
  render(<RequestSheet sessionId="session_test" turn={1} step={1} open onOpenChange={jest.fn()} />);
  return screen.getByRole("dialog", { name: "Model request" });
}

it("replaces the collapsed preview with a single copy of the expanded message", () => {
  const dialog = renderSheet();
  const row = within(dialog).getByRole("button", { name: /Read the attached report/ });
  fireEvent.click(row);

  expect(row).toHaveAttribute("aria-expanded", "true");
  expect(row).not.toHaveTextContent("Read the attached report.");
  expect(within(dialog).getAllByText("Read the attached report.")).toHaveLength(1);

  fireEvent.click(row);
  expect(row).toHaveTextContent("Read the attached report.");
});

it("shows stored images and downloads without fetching external images", () => {
  const dialog = renderSheet();
  fireEvent.click(within(dialog).getByRole("button", { name: /Read the attached report/ }));

  expect(within(dialog).getByRole("img", { name: "chart.png" })).toHaveAttribute(
    "src",
    `/api/v1/images/${imageId}/thumbnail`,
  );
  expect(within(dialog).getByRole("link", { name: /report.pdf/ })).toHaveAttribute(
    "href",
    `/api/v1/files/${fileId}`,
  );
  expect(dialog.querySelector('img[src*="external.example"]')).not.toBeInTheDocument();
});
