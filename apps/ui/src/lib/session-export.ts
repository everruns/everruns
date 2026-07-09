// Session export download flow shared by session lists.
//
// Decisions:
// - ATIF limit signals are user-facing: 413 (document over the server size
//   cap) and `X-Atif-Images-Omitted` become toasts. Everything else keeps the
//   pre-ATIF behavior (console.error only), so the flow degrades gracefully
//   against servers without ATIF support.
// - When the server advertises SEGMENTED export in its 413 (the RFC-9457
//   `detail` names `segmented=true`), the too-large case becomes recoverable:
//   we offer a "Download in parts" action that walks the segmented export and
//   saves each segment as a separate file. Against an older server whose 413
//   does not mention segmentation, we fall back to today's plain error toast.

import {
  exportSession,
  exportSessionSegmented,
  MAX_ATIF_SEGMENTS,
  SegmentedExportError,
  type SessionExportFormat,
} from "@/lib/api/sessions";
import { ApiError } from "@/lib/api/client";
import {
  formatAtifExportStopped,
  formatAtifImagesOmitted,
  formatAtifPartsExported,
  formatMessage,
  type SupportedLocale,
} from "@/lib/i18n";

export interface SessionExportToast {
  title: string;
  body: string;
  action?: { label: string; onClick: () => void | Promise<void> };
}

/** True when a 413 body indicates the server supports segmented export. */
function serverOffersSegmentation(error: ApiError): boolean {
  // The new server's 413 `detail` names `segmented=true`; an old server's does
  // not. The detail text is the only reliable discriminator — the `code` is
  // `atif_export_too_large` on both old and new servers, so it cannot gate the
  // offer (offering against an old server would only 400/404 on the walk).
  return /segmented/i.test(error.message ?? "");
}

/**
 * Walk the segmented ATIF export, saving each segment as its own file, and
 * surface progress/completion/failure via `notify`.
 */
async function runSegmentedExport(
  sessionId: string,
  locale: SupportedLocale,
  notify: (toast: SessionExportToast) => void,
): Promise<void> {
  try {
    const { partsSaved, imagesOmitted } = await exportSessionSegmented(sessionId);
    notify({
      title: formatMessage(locale, "atif_export_title"),
      body: formatAtifPartsExported(locale, partsSaved),
    });
    if (imagesOmitted > 0) {
      notify({
        title: formatMessage(locale, "atif_export_title"),
        body: formatAtifImagesOmitted(locale, imagesOmitted),
      });
    }
  } catch (error) {
    if (error instanceof SegmentedExportError) {
      const body =
        error.reason === "limit"
          ? formatMessage(locale, "atif_export_segmented_limit", { count: MAX_ATIF_SEGMENTS })
          : formatAtifExportStopped(locale, error.partsSaved);
      notify({ title: formatMessage(locale, "atif_export_failed_title"), body });
      return;
    }
    console.error("Failed to export session in segments:", error);
    notify({
      title: formatMessage(locale, "atif_export_failed_title"),
      body: formatMessage(locale, "atif_export_too_large"),
    });
  }
}

/**
 * Download a session export and surface ATIF limit alerts via `notify`.
 * `notify` is optional so callers outside the notifications provider keep the
 * plain download behavior.
 */
export async function downloadSessionExport(
  sessionId: string,
  format: SessionExportFormat,
  locale: SupportedLocale,
  notify?: (toast: SessionExportToast) => void,
): Promise<void> {
  try {
    const result = await exportSession(sessionId, format);
    if (format === "atif" && result.imagesOmitted > 0) {
      notify?.({
        title: formatMessage(locale, "atif_export_title"),
        body: formatAtifImagesOmitted(locale, result.imagesOmitted),
      });
    }
  } catch (error) {
    if (format === "atif" && error instanceof ApiError && error.status === 413) {
      // Recoverable path: offer a segmented download when the server supports
      // it and we can drive the walk (needs `notify` to confirm + report).
      if (notify && serverOffersSegmentation(error)) {
        notify({
          title: formatMessage(locale, "atif_export_title"),
          body: formatMessage(locale, "atif_export_segmented_prompt"),
          action: {
            label: formatMessage(locale, "atif_export_download_parts"),
            onClick: () => runSegmentedExport(sessionId, locale, notify),
          },
        });
        return;
      }
      // Old server (no segmentation) or no notify: plain error toast.
      // ApiError falls back to "API Error: <status> <statusText>" when the
      // response body had no parseable message; prefer the localized fallback.
      const genericMessage = `API Error: ${error.status} ${error.statusText}`;
      const body =
        error.message && error.message !== genericMessage
          ? error.message
          : formatMessage(locale, "atif_export_too_large");
      notify?.({ title: formatMessage(locale, "atif_export_failed_title"), body });
      return;
    }
    console.error("Failed to export session:", error);
  }
}
