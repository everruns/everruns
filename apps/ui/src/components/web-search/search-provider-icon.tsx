// Icons for web search providers. Decision: Lucide marks, not new Facet
// glyphs: a globe for Web search, a magnifier for free search, sparkles for
// the model's own search and a shield for Brave (its mark is a lion in a
// shield; the brand logo is not ours to redraw). Unknown connectors fall back
// to the connector's own icon name.

import { Globe, Search, ShieldCheck, Sparkles, Waypoints } from "lucide-react";
import { cn } from "@/lib/utils";

const ACCOUNT_ICONS: Record<string, typeof Globe> = {
  brave_search: ShieldCheck,
  parallel: Waypoints,
};

export function SearchProviderIcon({
  kind,
  provider,
  className,
}: {
  kind: "web_search" | "native" | "free" | "account";
  /** Connector id for accounts, such as `brave_search`. */
  provider?: string;
  className?: string;
}) {
  let Icon: typeof Globe = Search;
  if (kind === "web_search") Icon = Globe;
  else if (kind === "native") Icon = Sparkles;
  else if (kind === "account" && provider) Icon = ACCOUNT_ICONS[provider] ?? Search;
  return <Icon aria-hidden="true" className={cn("size-4 shrink-0", className)} />;
}
