import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Shorten a prefixed ID for display (like GitHub's short commit hashes).
 * Format: `{prefix}_{hex}` → `{prefix}_{first 7 hex chars}...`
 * Example: `session_01933b5a00007000...` → `session_01933b5...`
 *
 * @param id The full prefixed ID
 * @param hexChars Number of hex characters to show after prefix (default: 7, like GitHub)
 */
export function shortenId(id: string, hexChars: number = 7): string {
  const underscoreIndex = id.indexOf("_");
  if (underscoreIndex === -1) {
    // No prefix, just truncate
    return id.length > hexChars ? id.slice(0, hexChars) + "..." : id;
  }
  const prefix = id.slice(0, underscoreIndex + 1); // includes underscore
  const hex = id.slice(underscoreIndex + 1);
  if (hex.length <= hexChars) {
    return id; // Already short enough
  }
  return prefix + hex.slice(0, hexChars) + "...";
}
