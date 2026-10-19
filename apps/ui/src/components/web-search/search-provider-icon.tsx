// Icons for web search providers. Decision: Lucide marks, not new Facet
// glyphs: a globe for Web search, a magnifier for free search, sparkles for
// the model's own search, a shield for Brave (its mark is a lion in a
// shield; the brand logo is not ours to redraw), a cloud for Cloudflare and
// a badge check for the deployment's own (hosted) search.
// Unknown connectors fall back to the magnifier.

import { BadgeCheck, Cloud, Globe, Search, ShieldCheck, Sparkles, Waypoints } from "lucide-react";
import { cn } from "@/lib/utils";

const ACCOUNT_ICONS: Record<string, typeof Globe> = {
  brave_search: ShieldCheck,
  parallel: Waypoints,
  cloudflare_web_search: Cloud,
};

export function SearchProviderIcon({
  kind,
  provider,
  className,
}: {
  kind: "web_search" | "native" | "hosted" | "free" | "account";
  /** Connector id for accounts, such as `brave_search`. */
  provider?: string;
  className?: string;
}) {
  let Icon: typeof Globe = Search;
  if (kind === "web_search") Icon = Globe;
  else if (kind === "native") Icon = Sparkles;
  else if (kind === "hosted") Icon = BadgeCheck;
  else if (kind === "account" && provider) Icon = ACCOUNT_ICONS[provider] ?? Search;
  return <Icon aria-hidden="true" className={cn("size-4 shrink-0", className)} />;
}
