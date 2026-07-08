// Session export download flow shared by session lists.
//
// Decisions:
// - ATIF limit signals are user-facing: 413 (document over the server size
//   cap) becomes an error toast and `X-Atif-Images-Omitted` becomes an info
//   toast. Everything else keeps the pre-ATIF behavior (console.error only),
//   so the flow degrades gracefully against servers without ATIF support.

import { exportSession, type SessionExportFormat } from "@/lib/api/sessions";
import { ApiError } from "@/lib/api/client";
import { formatAtifImagesOmitted, formatMessage, type SupportedLocale } from "@/lib/i18n";

export interface SessionExportToast {
  title: string;
  body: string;
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
