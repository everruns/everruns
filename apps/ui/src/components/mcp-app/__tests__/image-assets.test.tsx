import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiAssetContext } from "@/hooks/use-api-asset";
import { MessageImage } from "@/components/chat/image-attachments";
import { setBlobDownloadHandler } from "@/lib/download";

const imageBytes = new Uint8Array([0, 255, 128, 65]);
const fetchImage = jest.fn(async () => ({
  ok: true,
  blob: async () => new Blob([imageBytes], { type: "image/png" }),
}));

function showImage(embedded: boolean) {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <ApiAssetContext value={embedded}>
        <MessageImage imageId="img_proof" filename="proof.png" />
      </ApiAssetContext>
    </QueryClientProvider>,
  );
}

const nativeFetch = global.fetch;
afterEach(() => {
  global.fetch = nativeFetch;
  fetchImage.mockClear();
});

it("keeps native image URLs outside embedded workspaces", () => {
  global.fetch = fetchImage as unknown as typeof fetch;
  showImage(false);
  expect(screen.getByAltText("proof.png")).toHaveAttribute(
    "src",
    "/api/v1/images/img_proof/thumbnail",
  );
  expect(fetchImage).not.toHaveBeenCalled();
});

it("renders transported thumbnail/full bytes and surfaces host save failures", async () => {
  global.fetch = fetchImage as unknown as typeof fetch;
  const restore = setBlobDownloadHandler(async () => {
    throw new Error("Host declined the image save");
  });
  try {
    showImage(true);
    await waitFor(() =>
      expect(screen.getByAltText("proof.png")).toHaveAttribute(
        "src",
        "data:image/png;base64,AP+AQQ==",
      ),
    );
    expect(fetchImage).toHaveBeenCalledWith("/api/v1/images/img_proof/thumbnail");
    fireEvent.click(screen.getByRole("button", { name: "proof.png" }));
    await waitFor(() => expect(screen.getAllByAltText("proof.png")).toHaveLength(2));
    await waitFor(() =>
      expect(screen.getAllByAltText("proof.png")[1]).toHaveAttribute(
        "src",
        "data:image/png;base64,AP+AQQ==",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    await screen.findByText("Host declined the image save");
    expect(fetchImage).toHaveBeenCalledWith("/api/v1/images/img_proof");
  } finally {
    setBlobDownloadHandler(restore);
  }
});
