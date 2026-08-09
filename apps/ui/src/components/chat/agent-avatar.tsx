/**
 * Avatar for a thread's counterpart. Agents and harnesses carry no image, so
 * identity is the initials of the display name over the brand accent — stable
 * per counterpart, and legible at the sizes the thread header and thread list
 * use.
 */
"use client";

import { Bot } from "lucide-react";
import { cn } from "@/lib/utils";

export function agentInitials(name: string): string {
  const words = name
    .split(/[\s_-]+/)
    .map((word) => word.trim())
    .filter(Boolean);
  if (words.length === 0) return "";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return `${words[0][0]}${words[1][0]}`.toUpperCase();
}

export function AgentAvatar({
  name,
  size = "md",
  className,
}: {
  name?: string | null;
  size?: "sm" | "md";
  className?: string;
}) {
  const initials = agentInitials(name ?? "");

  return (
    <span
      data-slot="agent-avatar"
      aria-hidden="true"
      className={cn(
        "inline-flex flex-none items-center justify-center border border-border/70 bg-accent/20 font-semibold text-foreground",
        size === "md" ? "size-8 text-[11px]" : "size-6 text-[10px]",
        className,
      )}
    >
      {initials || <Bot className={size === "md" ? "size-4" : "size-3"} />}
    </span>
  );
}
