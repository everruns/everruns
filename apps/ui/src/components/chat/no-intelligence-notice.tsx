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

/** One wording for every surface, so hosts that frame their own empty state
 *  (the chats list, the new-chat page) say the same thing this message does. */
export const NO_INTELLIGENCE_TITLE = "No intelligence available";
export const NO_INTELLIGENCE_DESCRIPTION =
  "This organisation has no model available for chat. Connect a provider and enable at least one model to start a conversation.";

/** The way out, or who to ask when the caller has no route to Settings. */
export function NoIntelligenceAction({ canManage }: { canManage: boolean }) {
  if (!canManage) {
    return (
      <p className="text-xs text-muted-foreground">
        Ask an organisation owner or admin to configure a model provider.
      </p>
    );
  }
  return (
    <Link href="/settings/providers" className={buttonVariants({ variant: "outline", size: "sm" })}>
      Manage providers &amp; models
    </Link>
  );
}

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
      <p className="text-sm font-semibold text-foreground">{NO_INTELLIGENCE_TITLE}</p>
      <p className="text-xs leading-relaxed text-muted-foreground">{NO_INTELLIGENCE_DESCRIPTION}</p>
      <div className="mt-1">
        <NoIntelligenceAction canManage={canManage} />
      </div>
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
