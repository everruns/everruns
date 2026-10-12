"use client";

import type { ComponentType } from "react";
import { Cloud, Search, LinkIcon } from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { getCapabilityIcon } from "@/lib/capability-icons";
import { McpServerMark } from "@/components/connections/mcp-server-mark";
import type { McpServerIcon } from "@/lib/api/mcp-server-types";

const iconMap: Record<string, ComponentType<{ className?: string }>> = {
  github: Github,
  cloud: Cloud,
  search: Search,
  daytona: getCapabilityIcon("daytona"),
};

export function ProviderIcon({
  iconName,
  iconUrl,
  icons,
  className,
}: {
  iconName: string;
  iconUrl?: string | null;
  icons?: McpServerIcon[] | null;
  className?: string;
}) {
  const Icon = iconMap[iconName] ?? LinkIcon;
  return (
    <McpServerMark
      icons={icons}
      iconUrl={iconUrl}
      className={className}
      fallback={<Icon className={className} />}
    />
  );
}
