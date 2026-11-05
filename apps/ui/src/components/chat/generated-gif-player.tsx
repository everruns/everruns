/**
 * Decisions:
 * - `generate_image` / `edit_image` return a GIF twice: the stored artifact
 *   (the animation) and a PNG of its first frame for the chat model. People
 *   should see the animation, so a GIF with an artifact plays at full size
 *   from `/v1/images/{id}` and its first-frame thumbnail is hidden.
 * - Pause swaps the animation for that first-frame PNG: browsers cannot pause
 *   an `<img>` GIF, and the still is already in the result.
 * - Only artifact ids from the tool's own JSON reach a URL, and only through
 *   `getImageUrl`, so a result cannot point the player at another origin.
 * - In chat the turn's steps fold away, but a generated image is the answer,
 *   so `GeneratedImageCard` shows it outside the fold, above the reply.
 */
"use client";

import { useState } from "react";
import { Download, ImageIcon, Pause, Play } from "lucide-react";
import type { ReactNode } from "react";
import type { Event, ToolCompletedData } from "@/lib/api/types";
import { extractResultImages, getFullText } from "./tool-call-utils";
import { getImageUrl } from "@/lib/api/images";
import { useUiTranslations } from "@/providers/locale-provider";

export const IMAGE_TOOLS = new Set(["generate_image", "edit_image"]);
const ARTIFACT_ID = /^[A-Za-z0-9_-]{1,80}$/;

export interface GeneratedGif {
  artifactId: string;
  filename: string;
  frames: number | null;
  frameDurationMs: number | null;
}

/** GIF artifacts in an image tool's result, in result order. */
export function extractGeneratedGifs(toolName: string, resultText: string): GeneratedGif[] {
  if (!IMAGE_TOOLS.has(toolName) || !resultText.trimStart().startsWith("{")) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(resultText);
  } catch {
    return [];
  }
  const images = (parsed as { images?: unknown })?.images;
  if (!Array.isArray(images)) return [];
  return images.flatMap((image): GeneratedGif[] => {
    if (!image || typeof image !== "object") return [];
    const entry = image as Record<string, unknown>;
    if (entry.media_type !== "image/gif") return [];
    if (typeof entry.artifact_id !== "string" || !ARTIFACT_ID.test(entry.artifact_id)) return [];
    return [
      {
        artifactId: entry.artifact_id,
        filename: typeof entry.filename === "string" ? entry.filename : "animation.gif",
        frames: typeof entry.frames === "number" ? entry.frames : null,
        frameDurationMs:
          typeof entry.frame_duration_ms === "number" ? entry.frame_duration_ms : null,
      },
    ];
  });
}

export function GeneratedGifPlayer({ gif, still }: { gif: GeneratedGif; still?: string }) {
  const ui = useUiTranslations();
  const [paused, setPaused] = useState(false);
  const src = getImageUrl(gif.artifactId);
  const loopSeconds =
    gif.frames && gif.frameDurationMs ? (gif.frames * gif.frameDurationMs) / 1000 : null;
  const badge = [
    "GIF",
    gif.frames ? `${gif.frames} ${ui("frames")}` : null,
    loopSeconds ? `${loopSeconds.toFixed(1)} s ${ui("loop")}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const canPause = Boolean(still);

  return (
    <figure
      className="inline-flex max-w-full flex-col overflow-hidden rounded-[8px] border border-border/70 bg-muted/30"
      data-testid="generated-gif"
    >
      <div className="relative">
        {/* eslint-disable-next-line @next/next/no-img-element -- stored image artifact. */}
        <img
          src={paused && still ? still : src}
          alt={gif.filename}
          loading="lazy"
          className="block max-h-80 w-auto max-w-full object-contain"
        />
        <span className="absolute left-2 top-2 rounded-[6px] bg-background/85 px-1.5 py-0.5 text-[11px] font-medium text-foreground shadow-sm">
          {badge}
        </span>
      </div>
      <figcaption className="flex items-center gap-3 border-t border-border/70 px-2.5 py-1.5 text-xs text-muted-foreground">
        <span className="min-w-0 flex-1 truncate">{gif.filename}</span>
        {canPause && (
          <button
            type="button"
            onClick={() => setPaused((current) => !current)}
            className="inline-flex items-center gap-1 transition-colors hover:text-foreground"
          >
            {paused ? <Play className="h-3.5 w-3.5" /> : <Pause className="h-3.5 w-3.5" />}
            {paused ? ui("Play") : ui("Pause")}
          </button>
        )}
        <a
          href={src}
          download={gif.filename}
          className="inline-flex items-center gap-1 transition-colors hover:text-foreground"
        >
          <Download className="h-3.5 w-3.5" />
          {ui("Download")}
        </a>
      </figcaption>
    </figure>
  );
}

interface GeneratedImageSummary {
  model: string | null;
  images: { artifactId: string; filename: string; mediaType: string; sizeBytes: number | null }[];
}

function parseImageResult(resultText: string): GeneratedImageSummary | null {
  if (!resultText.trimStart().startsWith("{")) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(resultText);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object") return null;
  const record = parsed as Record<string, unknown>;
  const images = Array.isArray(record.images) ? record.images : [];
  return {
    model: typeof record.model === "string" ? record.model : null,
    images: images.flatMap((image) => {
      if (!image || typeof image !== "object") return [];
      const entry = image as Record<string, unknown>;
      if (typeof entry.artifact_id !== "string" || !ARTIFACT_ID.test(entry.artifact_id)) return [];
      if (typeof entry.media_type !== "string" || !entry.media_type.startsWith("image/")) return [];
      if (entry.media_type === "image/svg+xml") return [];
      return [
        {
          artifactId: entry.artifact_id,
          filename: typeof entry.filename === "string" ? entry.filename : "image",
          mediaType: entry.media_type,
          sizeBytes: typeof entry.size_bytes === "number" ? entry.size_bytes : null,
        },
      ];
    }),
  };
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** Whether a finished tool call produced images worth showing outside the fold. */
export function hasGeneratedImages(toolName: string, result: ToolCompletedData | undefined) {
  if (!IMAGE_TOOLS.has(toolName) || !result) return false;
  return (parseImageResult(getFullText(result.result))?.images.length ?? 0) > 0;
}

/** A finished `generate_image` / `edit_image` call: GIFs play, stills show full size. */
export function GeneratedImageCard({
  toolName,
  result,
}: {
  toolName: string;
  result: ToolCompletedData;
}) {
  const ui = useUiTranslations();
  const text = getFullText(result.result);
  const summary = parseImageResult(text);
  if (!summary || summary.images.length === 0) return null;
  const gifs = extractGeneratedGifs(toolName, text);
  const stills = extractResultImages(result.result);
  const seconds = result.duration_ms ? `${(result.duration_ms / 1000).toFixed(1)} s` : null;
  const first = summary.images[0];

  return (
    <section
      className="max-w-2xl overflow-hidden rounded-[8px] border border-border/70 bg-background"
      data-testid="generated-image-card"
    >
      <header className="flex items-center gap-2 border-b border-border/70 px-3 py-2 text-xs">
        <ImageIcon className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="font-medium text-foreground">{ui("Generated image")}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">
          {[summary.model, seconds].filter(Boolean).join(" · ")}
        </span>
        {summary.images.length === 1 && (
          <span className="shrink-0 text-muted-foreground">
            {[first.filename, first.sizeBytes ? formatBytes(first.sizeBytes) : null]
              .filter(Boolean)
              .join(" · ")}
          </span>
        )}
      </header>
      <div className="flex flex-wrap gap-3 p-3">
        {summary.images.map((image, index) => {
          const gif = gifs.find((candidate) => candidate.artifactId === image.artifactId);
          if (gif) {
            return <GeneratedGifPlayer key={image.artifactId} gif={gif} still={stills[index]} />;
          }
          return (
            // eslint-disable-next-line @next/next/no-img-element -- stored image artifact.
            <img
              key={image.artifactId}
              src={getImageUrl(image.artifactId)}
              alt={image.filename}
              loading="lazy"
              className="block max-h-80 w-auto max-w-full rounded-[6px] border border-border/70 object-contain"
            />
          );
        })}
      </div>
    </section>
  );
}

/** Cards for a turn's finished image tool calls, shown outside the work-log fold. */
export function generatedImageCards(events: Event[]): ReactNode[] {
  return events.flatMap((event) => {
    if (event.type !== "tool.completed") return [];
    const result = event.data as unknown as ToolCompletedData;
    if (!hasGeneratedImages(result.tool_name, result)) return [];
    return [
      <GeneratedImageCard key={result.tool_call_id} toolName={result.tool_name} result={result} />,
    ];
  });
}

/**
 * What a turn keeps visible outside its folded work log: live questions while
 * it runs (`interactive`, null once done), then its generated images.
 */
export function turnAttention(
  events: Event[],
  interactive: ((event: Event) => ReactNode) | null,
): ReactNode[] {
  return [
    ...(interactive ? events.map(interactive).filter(Boolean) : []),
    ...generatedImageCards(events),
  ];
}
