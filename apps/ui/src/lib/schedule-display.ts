/**
 * Schedule display helpers shared by chat activity rows and schedule pages.
 *
 * Decisions:
 * - Favor compact labels for common recurring patterns because schedule chips need to fit inline.
 * - Keep the parser deliberately narrow to the 5-field session-schedule cron shape we support today.
 * - Fall back to raw cron for patterns we do not recognize instead of guessing incorrectly.
 */

const WEEKDAY_LABELS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const BACKGROUND_MONITOR_TITLE_LINE = /^- title: `([^`]+)`$/m;
const BACKGROUND_MONITOR_HEADER = /^Monitor: (.+)$/m;

function formatClockTime(hour: number, minute: number): string {
  return new Intl.DateTimeFormat("en-US", {
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
    timeZone: "UTC",
  }).format(new Date(Date.UTC(2026, 0, 1, hour, minute)));
}

function formatWeekdays(value: string): string | null {
  if (value === "*") return null;
  if (value === "1-5") return "Weekdays";
  if (value === "0,6" || value === "6,0") return "Weekends";

  const parts = value.split(",");
  const labels = parts
    .map((part) => {
      const day = Number(part);
      return Number.isInteger(day) && day >= 0 && day <= 6 ? WEEKDAY_LABELS[day] : null;
    })
    .filter((label): label is string => Boolean(label));

  if (labels.length !== parts.length || labels.length === 0) return null;
  return labels.join(", ");
}

export function formatScheduleCadence(input: {
  cronExpression?: string;
  scheduledAt?: string;
  timezone?: string;
}): string | null {
  const cronExpression = input.cronExpression?.trim();
  if (cronExpression) {
    const fields = cronExpression.split(/\s+/);
    if (fields.length !== 5) return cronExpression;

    const [minute, hour, dayOfMonth, month, dayOfWeek] = fields;

    if (minute === "*" && hour === "*" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*")
      return "Every minute";

    const everyMinutes = minute.match(/^\*\/(\d+)$/);
    if (everyMinutes && hour === "*" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*") {
      const interval = Number(everyMinutes[1]);
      return interval === 1 ? "Every minute" : `Every ${interval}m`;
    }

    if (minute === "0" && hour === "*" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*")
      return "Hourly";

    const everyHours = hour.match(/^\*\/(\d+)$/);
    if (
      everyHours &&
      /^\d+$/.test(minute) &&
      dayOfMonth === "*" &&
      month === "*" &&
      dayOfWeek === "*"
    ) {
      const interval = Number(everyHours[1]);
      return `Every ${interval}h`;
    }

    if (
      /^\d+$/.test(minute) &&
      /^\d+$/.test(hour) &&
      dayOfMonth === "*" &&
      month === "*" &&
      dayOfWeek === "*"
    ) {
      return `Daily ${formatClockTime(Number(hour), Number(minute))}`;
    }

    const weekdayLabel = formatWeekdays(dayOfWeek);
    if (
      weekdayLabel &&
      /^\d+$/.test(minute) &&
      /^\d+$/.test(hour) &&
      dayOfMonth === "*" &&
      month === "*"
    ) {
      return `${weekdayLabel} ${formatClockTime(Number(hour), Number(minute))}`;
    }

    return cronExpression;
  }

  if (input.scheduledAt) {
    return `At ${formatScheduledDateTime(input.scheduledAt, input.timezone)}`;
  }

  return null;
}

export function formatScheduledDateTime(dateString: string, timezone = "UTC"): string {
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZone: timezone,
    timeZoneName: "short",
  }).format(new Date(dateString));
}

export function getScheduleDisplayTitle(description: string): string {
  const trimmed = description.trim();
  const monitorHeader = trimmed.match(BACKGROUND_MONITOR_HEADER)?.[1]?.trim();
  if (monitorHeader) return monitorHeader;

  const monitorTitle = trimmed.match(BACKGROUND_MONITOR_TITLE_LINE)?.[1]?.trim();
  if (monitorTitle) return monitorTitle;

  return trimmed;
}
