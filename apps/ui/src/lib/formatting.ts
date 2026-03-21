/**
 * Formatting utilities for displaying numbers, dates, and durations.
 * Centralized to avoid duplication across components.
 */

/**
 * Format a number in compact form (e.g., 1000 → "1.0K", 1500000 → "1.5M")
 */
export function formatCompactNumber(value: number): string {
  if (value >= 1000000) return `${(value / 1000000).toFixed(1)}M`;
  if (value >= 1000) return `${(value / 1000).toFixed(1)}K`;
  return value.toString();
}

/**
 * Format token count in compact form
 * @alias formatCompactNumber - provided for semantic clarity
 */
export const formatTokens = formatCompactNumber;

/**
 * Format milliseconds duration as human-readable string
 * @param ms Duration in milliseconds
 * @returns Formatted string (e.g., "1.23s", "456ms")
 */
export function formatDuration(ms: number): string {
  if (ms >= 1000) {
    return `${(ms / 1000).toFixed(2)}s`;
  }
  return `${ms}ms`;
}

/**
 * Format seconds duration as human-readable string
 * @param seconds Duration in seconds
 * @returns Formatted string (e.g., "45s", "2m 30s")
 */
export function formatDurationSeconds(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes}m ${remainingSeconds}s`;
}

/**
 * Format a date as relative time (e.g., "5 min ago", "2 hours ago", "yesterday")
 */
export function formatRelativeTime(dateString: string): string {
  const date = new Date(dateString);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSecs = Math.floor(diffMs / 1000);
  const diffMins = Math.floor(diffSecs / 60);
  const diffHours = Math.floor(diffMins / 60);
  const diffDays = Math.floor(diffHours / 24);

  if (diffSecs < 60) return "just now";
  if (diffMins < 60) return `${diffMins} min ago`;
  if (diffHours < 24) return `${diffHours} hour${diffHours > 1 ? "s" : ""} ago`;
  if (diffDays === 1) return "yesterday";
  if (diffDays < 7) return `${diffDays} days ago`;
  return date.toLocaleDateString();
}

/**
 * Format a date for display using locale settings
 */
export function formatDate(dateString: string): string {
  return new Date(dateString).toLocaleString();
}

/**
 * Format a date as short date (without time)
 */
export function formatShortDate(dateString: string): string {
  return new Date(dateString).toLocaleDateString();
}
