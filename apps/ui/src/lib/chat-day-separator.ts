/**
 * Day separators in the chat transcript (knowledge/ui/chat-experience.md).
 *
 * Decision: a centered timestamp goes before a user turn that opens the
 * transcript, follows the previous user turn by more than an hour, or lands
 * on a different calendar day. Closer turns read as one conversation and get
 * none.
 */

export const DAY_SEPARATOR_GAP_MS = 60 * 60 * 1000;

function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** Whether a separator goes before a user turn at `ms`, after one at `previousMs`. */
export function needsDaySeparator(previousMs: number | undefined, ms: number): boolean {
  if (Number.isNaN(ms)) return false;
  if (previousMs == null || Number.isNaN(previousMs)) return true;
  return ms - previousMs > DAY_SEPARATOR_GAP_MS || startOfDay(ms) !== startOfDay(previousMs);
}

export interface DaySeparatorWords {
  today: string;
  yesterday: string;
}

/** "Today 7:40 PM", "Yesterday 9:02 AM", "Wednesday 7:53 PM", then a date for older turns. */
export function formatDaySeparator(
  ms: number,
  nowMs: number,
  locale: string,
  words: DaySeparatorWords,
): string {
  const time = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" }).format(ms);
  const days = Math.round((startOfDay(nowMs) - startOfDay(ms)) / (24 * 60 * 60 * 1000));
  if (days === 0) return `${words.today} ${time}`;
  if (days === 1) return `${words.yesterday} ${time}`;
  if (days > 1 && days < 7) {
    const weekday = new Intl.DateTimeFormat(locale, { weekday: "long" }).format(ms);
    return `${weekday} ${time}`;
  }
  const sameYear = new Date(ms).getFullYear() === new Date(nowMs).getFullYear();
  const date = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
  }).format(ms);
  return `${date} ${time}`;
}
