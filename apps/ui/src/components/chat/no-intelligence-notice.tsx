"use client";

/**
 * "This org has no model to think with" — shown on every chat surface when no
 * enabled, healthy chat model exists.
 *
 * Decisions:
 * - A centred message, not a warning stripe. The chat is unusable, so this is
 *   the state of the surface rather than an aside about it; weight (bold) and
 *   position carry the message instead of an alert colour.
 * - The route out is only rendered for a caller who holds `provider.manage`.
 *   Everyone else gets a sentence naming who can fix it, because a link they
 *   cannot act on reads as a dead end.
 * - Silent while loading and silent on a failed read: a false "no intelligence"
 *   on a working org is worse than a late one.
 */

import Link from "next/link";
import { Sparkles } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { useIntelligenceStatus } from "@/hooks/use-intelligence";
import { cn } from "@/lib/utils";

export function NoIntelligenceNotice({ className }: { className?: string }) {
  const { isLoading, available, canManage } = useIntelligenceStatus();

  if (isLoading || available) return null;

  return (
    <div
      role="status"
      className={cn(
        "mx-auto flex max-w-md flex-col items-center gap-2 border border-border/70 bg-card/60 px-6 py-5 text-center",
        className,
      )}
    >
      <Sparkles className="icon-sharp size-5 text-muted-foreground" strokeWidth={1.8} />
      <p className="text-sm font-semibold text-foreground">No intelligence available</p>
      <p className="text-xs leading-relaxed text-muted-foreground">
        This organisation has no model available for chat. Connect a provider and enable at least
        one model to start a conversation.
      </p>
      {canManage ? (
        <Link
          href="/settings/providers"
          className={cn(buttonVariants({ variant: "outline", size: "sm" }), "mt-1")}
        >
          Manage providers &amp; models
        </Link>
      ) : (
        <p className="mt-1 text-xs text-muted-foreground">
          Ask an organisation owner or admin to configure a model provider.
        </p>
      )}
    </div>
  );
}
