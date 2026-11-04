type BlobDownload = (filename: string, blob: Blob) => Promise<void>;

// MCP App sandboxes forbid direct downloads. Their host owns the save action;
// the ordinary browser UI keeps its native download behavior.
let embeddedDownload: BlobDownload | undefined;

export class BlobDownloadError extends Error {}

export function setBlobDownloadHandler(handler: BlobDownload | undefined) {
  const previous = embeddedDownload;
  embeddedDownload = handler;
  return previous;
}

export async function downloadBlob(filename: string, blob: Blob): Promise<void> {
  if (embeddedDownload) {
    try {
      return await embeddedDownload(filename, blob);
    } catch (error) {
      throw new BlobDownloadError(error instanceof Error ? error.message : String(error));
    }
  }
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
