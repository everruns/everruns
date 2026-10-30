/**
 * Desktop recordings: the video of what an agent did on its Sandbox desktop,
 * shown under the turn that did it.
 *
 * When a Session with a recorded desktop goes idle, the server stops the
 * recording and saves it read-only under `/recordings/` in the Session's
 * files, announced with a `file.written` event. That happens just after the
 * turn ends, so a recording belongs to the last turn that ended at or before
 * its event, and renders on the same row as the turn divider.
 */
"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Download, MonitorPlay } from "lucide-react";
import type { Event } from "@/lib/api/types";
import { getApiBaseUrl } from "@/lib/api/client";
import { getLastVisibleEventIdByTurn } from "@/components/chat/turn-delimiter";
import { getTurnWindows } from "@/components/chat/run-cards";

const RECORDINGS_DIR = "/recordings/";
const VIDEO_EXTENSIONS = new Set(["mp4", "webm", "mkv", "mov"]);

export interface DesktopRecording {
  path: string;
  sizeBytes: number;
}

function recordingFromEvent(event: Event): DesktopRecording | null {
  if (event.type !== "file.written") return null;
  const data = event.data as { path?: unknown; size_bytes?: unknown } | null;
  const path = typeof data?.path === "string" ? data.path : null;
  if (!path?.startsWith(RECORDINGS_DIR)) return null;
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  if (!VIDEO_EXTENSIONS.has(extension)) return null;
  const sizeBytes = typeof data?.size_bytes === "number" ? data.size_bytes : 0;
  return { path, sizeBytes };
}

/** Group recordings by the transcript row of the turn that produced them. */
export function assignRecordingsToEvents(events: Event[]): Map<string, DesktopRecording[]> {
  const byEventId = new Map<string, DesktopRecording[]>();
  const recordings = events
    .map((event) => ({ event, recording: recordingFromEvent(event) }))
    .filter((entry): entry is { event: Event; recording: DesktopRecording } =>
      Boolean(entry.recording),
    );
  if (recordings.length === 0) return byEventId;

  const ended = getTurnWindows(events).filter((window) => window.endedAt != null);
  const lastVisibleEventIdByTurn = getLastVisibleEventIdByTurn(events);
  for (const { event, recording } of recordings) {
    const writtenAt = Date.parse(event.ts);
    if (Number.isNaN(writtenAt)) continue;
    let turnId: string | undefined;
    for (const window of ended) {
      if ((window.endedAt ?? Infinity) > writtenAt) break;
      turnId = window.turnId;
    }
    const eventId = turnId ? lastVisibleEventIdByTurn.get(turnId) : undefined;
    if (!eventId) continue;
    const existing = byEventId.get(eventId);
    if (existing) existing.push(recording);
    else byEventId.set(eventId, [recording]);
  }
  return byEventId;
}

/** Same-origin raw download of a session file, playable by `<video>`. */
export function sessionFileDownloadUrl(sessionId: string, path: string): string {
  const encoded = path.replace(/^\/+/, "").split("/").map(encodeURIComponent).join("/");
  return `${getApiBaseUrl()}/v1/sessions/${sessionId}/fs/_/download/${encoded}`;
}

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

export function DesktopRecordingCards({
  sessionId,
  recordings,
}: {
  sessionId: string;
  recordings: DesktopRecording[];
}) {
  const ui = useUiTranslations();
  if (recordings.length === 0) return null;
  return (
    <div role="list" aria-label={ui("Desktop recordings of this turn")}>
      {recordings.map((recording) => {
        const url = sessionFileDownloadUrl(sessionId, recording.path);
        return (
          <div
            role="listitem"
            key={recording.path}
            data-slot="desktop-recording"
            className="mt-2 border border-border/70 bg-card/90 text-[13px] shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]"
          >
            <div className="flex items-center gap-2 px-3 py-2">
              <MonitorPlay className="h-3.5 w-3.5 flex-none text-muted-foreground" />
              <span className="font-medium text-foreground">{ui("Desktop recording")}</span>
              <span className="truncate text-xs text-muted-foreground">
                {recording.path} · {formatSize(recording.sizeBytes)}
              </span>
              <a
                href={url}
                download
                className="ml-auto flex flex-none items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
              >
                <Download className="h-3.5 w-3.5" />
                {ui("Download")}
              </a>
            </div>
            {/* A silent screen capture has no speech to caption. */}
            {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
            <video
              aria-label={ui("Desktop recording {value0}", { value0: recording.path })}
              controls
              preload="metadata"
              src={url}
              className="block max-h-[480px] w-full bg-black"
            />
          </div>
        );
      })}
    </div>
  );
}
