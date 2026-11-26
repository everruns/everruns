/**
 * Stored portrait for a thread's counterpart, with display initials for
 * agents and harnesses that have no portrait.
 */
"use client";

import { AgentAvatar as StoredAgentAvatar } from "@/components/agents/agent-avatar";
import type { AgentAvatar as AgentAvatarData } from "@/lib/api/agent-types";
import { AgentIcon } from "@/components/icons/facet-icons";
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
  avatar,
  size = "md",
  className,
}: {
  name?: string | null;
  avatar?: AgentAvatarData | null;
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
      <StoredAgentAvatar
        avatar={avatar}
        size={size === "md" ? 32 : 24}
        fallback={initials || <AgentIcon className={size === "md" ? "size-4" : "size-3"} />}
      />
    </span>
  );
}
