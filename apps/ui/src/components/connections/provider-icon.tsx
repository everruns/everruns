"use client";

import { Github, Cloud, Search, LinkIcon } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { getCapabilityIcon } from "@/lib/capability-icons";

const iconMap: Record<string, LucideIcon> = {
  github: Github,
  cloud: Cloud,
  search: Search,
  daytona: getCapabilityIcon("daytona"),
};

export function ProviderIcon({ iconName, className }: { iconName: string; className?: string }) {
  const Icon = iconMap[iconName] ?? LinkIcon;
  return <Icon className={className} />;
}
