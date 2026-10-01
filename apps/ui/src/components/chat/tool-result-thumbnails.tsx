/**
 * Decisions:
 * - Image parts of a tool result (computer-use screenshots) show as small
 *   thumbnails under the row, without expanding it: the frame is what the
 *   agent acted on, and a reader scanning the session wants to see it.
 * - Sources come only from `extractResultImages`, which admits raster data
 *   URLs and http(s) URLs; nothing else reaches an `<img src>`.
 * - Full size opens in a dialog rather than a new tab, since data URLs cannot
 *   be opened as top-level navigations in most browsers.
 */
"use client";

import { useState } from "react";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { useLocale } from "@/providers/locale-provider";

export function ToolResultThumbnails({ images }: { images: string[] }) {
  const { t } = useLocale();
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  if (images.length === 0) return null;
  const open = openIndex === null ? null : images[openIndex];

  return (
    <>
      <div className="mt-1.5 flex flex-wrap gap-2" data-testid="tool-result-thumbnails">
        {images.map((src, index) => (
          <button
            key={`${index}-${src.slice(-24)}`}
            type="button"
            onClick={() => setOpenIndex(index)}
            aria-label={t("open_tool_result_image", { value: index + 1 })}
            className="block overflow-hidden rounded-[6px] border border-border/70 bg-muted/30 transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {/* eslint-disable-next-line @next/next/no-img-element -- tool output is a data URL or a remote image. */}
            <img
              src={src}
              alt={t("tool_result_image", { value: index + 1 })}
              loading="lazy"
              className="h-24 w-auto max-w-[180px] object-contain"
            />
          </button>
        ))}
      </div>

      <Dialog open={open !== null} onOpenChange={(next) => !next && setOpenIndex(null)}>
        <DialogContent className="max-h-[90vh] max-w-5xl overflow-hidden p-0">
          <DialogTitle className="sr-only">
            {t("tool_result_image", { value: (openIndex ?? 0) + 1 })}
          </DialogTitle>
          {open && (
            <div className="flex items-center justify-center overflow-auto bg-muted/30 p-4">
              {/* eslint-disable-next-line @next/next/no-img-element -- tool output is a data URL or a remote image. */}
              <img
                src={open}
                alt={t("tool_result_image", { value: (openIndex ?? 0) + 1 })}
                className="max-h-[80vh] max-w-full rounded object-contain"
              />
            </div>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
