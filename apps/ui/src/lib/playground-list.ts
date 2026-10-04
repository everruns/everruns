import type { PrincipalSummary, Session, SessionActivity } from "@/lib/api/types";

/** How the Playground library clusters the current page. */
export type PlaygroundGroupBy = "day" | "agent" | "none";

export type PlaygroundDay = "Today" | "Yesterday" | "Earlier";

/** `empty` is a list-only state: the session has no transcript yet. */
export type PlaygroundRowStatus = "empty" | SessionActivity;

const STATUS_LABELS: Record<PlaygroundRowStatus, string> = {
  empty: "No messages",
  running: "Running",
  paused: "Paused",
  failed: "Failed",
  completed: "Completed",
  idle: "Waiting for user",
};

const DAY_MS = 86_400_000;

export function playgroundStatus(
  session: Pick<Session, "activity" | "event_count" | "preview" | "output_preview">,
): PlaygroundRowStatus {
  const hasTranscript =
    (session.event_count ?? 0) > 0 ||
    Boolean(session.preview?.trim()) ||
    Boolean(session.output_preview?.trim());
  if (!hasTranscript) return "empty";
  return session.activity ?? "idle";
}

export function playgroundStatusLabel(status: PlaygroundRowStatus): string {
  return STATUS_LABELS[status];
}

/** Latest assistant text, then the opening user text. */
export function playgroundPreview(session: Pick<Session, "preview" | "output_preview">): string {
  return session.output_preview?.trim() || session.preview?.trim() || "No messages yet";
}

function startOfLocalDay(value: Date): number {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
}

export function playgroundDayLabel(iso: string, now: Date = new Date()): PlaygroundDay {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "Earlier";
  const diffDays = Math.floor((startOfLocalDay(now) - startOfLocalDay(date)) / DAY_MS);
  if (diffDays <= 0) return "Today";
  if (diffDays === 1) return "Yesterday";
  return "Earlier";
}

/** Clock time for today, "Yesterday", otherwise a short calendar date. */
export function playgroundRowTime(iso: string, now: Date = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const day = playgroundDayLabel(iso, now);
  if (day === "Today") {
    return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  }
  if (day === "Yesterday") return "Yesterday";
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function principalDisplayName(principal?: PrincipalSummary | null): string | null {
  const metadata = principal?.metadata;
  if (!metadata || typeof metadata !== "object") return null;
  const name = (metadata as { name?: unknown }).name;
  return typeof name === "string" && name.trim() ? name.trim() : null;
}

/** Two-letter mark: first letters of the first two words, or the first two letters. */
export function nameInitials(name: string): string {
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "";
  const first = parts[0]?.[0] ?? "";
  const second = parts.length > 1 ? (parts[1]?.[0] ?? "") : (parts[0]?.[1] ?? "");
  return `${first}${second}`.toUpperCase();
}

/** Human operator who started the chat, distinct from the virtual user it talks as. */
export function playgroundStarter(session: Pick<Session, "effective_owner" | "owner">): {
  name: string;
  initials: string;
} | null {
  const name = principalDisplayName(session.effective_owner) ?? principalDisplayName(session.owner);
  if (!name) return null;
  const initials = nameInitials(name);
  if (!initials) return null;
  return { name, initials };
}

export interface PlaygroundGroup<T> {
  label: string;
  rows: T[];
}

/**
 * Cluster one server page without re-sorting it. Search and filters stay on the
 * request. The page arrives newest-activity first, so the first time a day or
 * agent appears is the group a reader should see first.
 */
export function groupPlaygroundSessions<T extends { updatedAt: string; agentLabel: string }>(
  rows: T[],
  groupBy: PlaygroundGroupBy,
  now: Date = new Date(),
): PlaygroundGroup<T>[] {
  if (rows.length === 0) return [];
  if (groupBy === "none") return [{ label: "", rows }];
  const groups: PlaygroundGroup<T>[] = [];
  const index = new Map<string, PlaygroundGroup<T>>();
  for (const row of rows) {
    const label = groupBy === "day" ? playgroundDayLabel(row.updatedAt, now) : row.agentLabel;
    const existing = index.get(label);
    if (existing) {
      existing.rows.push(row);
      continue;
    }
    const group = { label, rows: [row] };
    index.set(label, group);
    groups.push(group);
  }
  return groups;
}
