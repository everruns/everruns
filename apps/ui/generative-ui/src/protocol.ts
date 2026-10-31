import { extractFenceOperations } from "@copilotkit/core";
import {
  getOperationSurfaceId,
  type A2UIOperation,
  type GenerativeUISurfaceContent,
} from "@copilotkit/shared";

export type UIMessage = { id: string; text: string; role?: string };
export type Segment = {
  type: "markdown" | "openui" | "a2ui";
  content: string;
  index: number;
  target?: string;
  closed: boolean;
};

// Track every Markdown fence, including ordinary code examples, before
// recognising widget languages. Preserve the original prose byte-for-byte.
export function splitUIBlocks(text: string): Segment[] {
  const segments: Segment[] = [];
  let offset = 0,
    lastEnd = 0;
  let active:
    | { marker: string; length: number; type?: "a2ui" | "openui"; start: number; target?: string }
    | undefined;
  const push = (type: Segment["type"], content: string, closed: boolean, target?: string) => {
    segments.push({ type, content, closed, target, index: segments.length });
  };
  for (const line of text.match(/[^\n]*(?:\n|$)/g) ?? []) {
    if (!line) continue;
    const bare = line.replace(/\r?\n$/, "");
    const fence = /^(`{3,}|~{3,})([^\r\n]*)\r?$/.exec(bare);
    if (active) {
      if (
        fence &&
        fence[1][0] === active.marker &&
        fence[1].length >= active.length &&
        !fence[2].trim()
      ) {
        if (active.type) {
          push(active.type, text.slice(active.start, offset), true, active.target);
          lastEnd = offset + line.length;
        }
        active = undefined;
      }
    } else if (fence) {
      const [language, ...info] = fence[2].trim().split(/\s+/);
      const type = language === "a2ui" || language === "openui" ? language : undefined;
      if (type && offset > lastEnd) push("markdown", text.slice(lastEnd, offset), true);
      active = {
        marker: fence[1][0],
        length: fence[1].length,
        type,
        start: offset + line.length,
        target:
          type === "a2ui" ? info.find((token) => token.startsWith("target="))?.slice(7) : undefined,
      };
    }
    offset += line.length;
  }
  if (active?.type) {
    push(active.type, text.slice(active.start), false, active.target);
    lastEnd = text.length;
  }
  if (lastEnd < text.length) push("markdown", text.slice(lastEnd), true);
  return segments;
}
export function isLegacy(code: string): boolean {
  try {
    const value = JSON.parse(code);
    return typeof value?.type === "string";
  } catch {
    return /^\s*\{\s*"type"\s*:/.test(code);
  }
}
export function blockKey(messageId: string, index: number) {
  return JSON.stringify([messageId, index]);
}

export function buildSurfaces(messages: readonly UIMessage[], streaming = false) {
  const surfaces = new Map<string, GenerativeUISurfaceContent>();
  const mounts = new Map<string, string[]>();
  const errors = new Map<string, string>();
  for (const message of messages) {
    if (message.role === "user") continue;
    for (const segment of splitUIBlocks(message.text)) {
      if (segment.type !== "a2ui") continue;
      const key = blockKey(message.id, segment.index);
      if (segment.content.length > 256 * 1024) {
        errors.set(key, "UI block exceeds the rendering limit");
        continue;
      }
      if (isLegacy(segment.content)) continue;
      const parsed = extractFenceOperations(segment.content);
      if ((parsed.invalidLines.length || !parsed.operations.length) && segment.closed)
        errors.set(key, "Invalid A2UI operations");
      for (const raw of parsed.operations) {
        const requested = getOperationSurfaceId(raw as A2UIOperation);
        if (!requested) {
          errors.set(key, "Operation has no surfaceId");
          continue;
        }
        // Inline surfaces belong to their reply. Named page targets may update across replies.
        const surfaceId = segment.target ? requested : `${message.id}:${requested}`;
        const kind = Object.keys(raw).find((k) =>
          ["createSurface", "updateComponents", "updateDataModel", "deleteSurface"].includes(k),
        );
        if (!kind) {
          errors.set(key, "Unsupported A2UI operation");
          continue;
        }
        const body = raw[kind];
        if (!body || typeof body !== "object") {
          errors.set(key, "Invalid A2UI operation");
          continue;
        }
        const operation = { ...raw, [kind]: { ...body, surfaceId } } as A2UIOperation;
        let surface = surfaces.get(surfaceId);
        if (!surface) {
          if (surfaces.size >= 100) {
            errors.set(key, "Too many UI surfaces");
            continue;
          }
          surface = {
            surfaceId,
            target: segment.target ?? `chat:${surfaceId}`,
            catalogId: "copilotkit/standard@1",
            rejected: [],
            status: "ready",
            operations: [],
          };
          surfaces.set(surfaceId, surface);
          if (!segment.target) mounts.set(key, [...(mounts.get(key) ?? []), surface.target]);
        }
        if ((surface.operations?.length ?? 0) >= 2000) {
          errors.set(key, "Too many UI operations");
          continue;
        }
        surface.operations!.push(operation);
        surface.status =
          streaming && message === messages.at(-1) && !segment.closed ? "streaming" : "ready";
      }
    }
  }
  return { surfaces, mounts, errors };
}

/** Nonnegative bar charts should compare magnitudes from zero unless authored otherwise. */
export function chartWithBaseline<
  T extends {
    chartType?: string;
    data?: Record<string, unknown>[];
    yAxis?: { min?: number; max?: number; label?: string; width?: number };
  },
>(props: T): T & { yAxis?: { min?: number; max?: number; label?: string; width?: number } } {
  if ((props.chartType ?? "bar") !== "bar" || props.yAxis?.min !== undefined) return props;
  const values = (props.data ?? []).flatMap((row) =>
    Object.values(row).filter(
      (value): value is number => typeof value === "number" && Number.isFinite(value),
    ),
  );
  return values.length && values.every((value) => value >= 0)
    ? { ...props, yAxis: { ...props.yAxis, min: 0 } }
    : props;
}
