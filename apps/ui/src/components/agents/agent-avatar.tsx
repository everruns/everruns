// Agent avatar image with an icon fallback. Square by default, matching the
// stored avatar; `shape="circle"` uses the server's pre-masked circular preset
// rather than CSS clipping, so it looks the same as on Slack and A2A clients.
import { AgentIcon } from "@/components/icons/facet-icons";
import type { ReactNode } from "react";

import { agentAvatarUrl } from "@/lib/api/agents";
import type { AgentAvatar as AgentAvatarData } from "@/lib/api/agent-types";
import { cn } from "@/lib/utils";

export function AgentAvatar({
  avatar,
  size,
  shape = "square",
  fallback = <AgentIcon size={size} />,
  className,
}: {
  avatar?: AgentAvatarData | null;
  /** Rendered edge in CSS pixels; a 2x preset is requested for sharpness. */
  size: number;
  shape?: "square" | "circle";
  fallback?: ReactNode;
  className?: string;
}) {
  if (!avatar) return <>{fallback}</>;
  return (
    // Preset PNGs are already sized server-side; next/image would add nothing.
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={agentAvatarUrl(avatar, size * 2, shape)}
      alt=""
      width={size}
      height={size}
      className={cn("flex-none object-cover", shape === "square" && "rounded-sm", className)}
      style={{ width: size, height: size }}
    />
  );
}
