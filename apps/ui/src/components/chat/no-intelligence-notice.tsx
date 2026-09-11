"use client";

/**
 * "This org has no model to think with" — shown on every chat surface when no
 * enabled, healthy chat model exists.
 *
 * Decisions:
 * - A centred message, not a warning stripe. The chat is unusable, so this is
 *   the state of the surface rather than an aside about it; weight (bold) and
 *   position carry the message instead of an alert colour.
 * - One message per surface. Where a surface already shows a centred empty
 *   state ("No messages yet", "No chats yet"), this *replaces* it rather than
 *   stacking under it — two centred cards competing for the same job reads as a
 *   bug. Hosts that already frame their own box pass `variant="plain"` so the
 *   message does not land as a card inside a card.
 * - The route out is only rendered for a caller who holds `provider.manage`.
 *   Everyone else gets a sentence naming who can fix it, because a link they
 *   cannot act on reads as a dead end.
 * - Silent while loading and silent on a failed read: a false "no intelligence"
 *   on a working org is worse than a late one.
 * - Split presentational/data-bound so `/dev/chat-components` can show both
 *   permission variants without a backend.
 */

import Link from "next/link";
import { Sparkles } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { useIntelligenceStatus } from "@/hooks/use-intelligence";
import { cn } from "@/lib/utils";

export function NoIntelligenceMessage({
  canManage,
  variant = "card",
  className,
}: {
  /** Render the route to Settings → Providers. */
  canManage: boolean;
  /** `plain` drops the box, for a host that already draws one. */
  variant?: "card" | "plain";
  className?: string;
}) {
  return (
    <div
      role="status"
      className={cn(
        "mx-auto flex max-w-md flex-col items-center gap-2 text-center",
        variant === "card" && "border border-border/70 bg-card/60 px-6 py-5",
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

export function NoIntelligenceNotice({
  variant,
  className,
}: {
  variant?: "card" | "plain";
  className?: string;
}) {
  const { isLoading, available, canManage } = useIntelligenceStatus();

  if (isLoading || available) return null;

  return <NoIntelligenceMessage canManage={canManage} variant={variant} className={className} />;
}
