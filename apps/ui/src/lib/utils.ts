import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Get the last N characters of an ID for compact display.
 * Uses last chars because they contain the random bits (more unique).
 * Example: `session_01933b5a00007000800000000000003` → `00000003`
 *
 * @param id The full prefixed ID
 * @param chars Number of characters to show (default: 8)
 */
export function shortenId(id: string, chars: number = 8): string {
  return id.slice(-chars);
}
