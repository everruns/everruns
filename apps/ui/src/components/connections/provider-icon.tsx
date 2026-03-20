"use client";

import type { ComponentType } from "react";
import { Github, Cloud, Search, LinkIcon } from "lucide-react";
import { getCapabilityIcon } from "@/lib/capability-icons";

const iconMap: Record<string, ComponentType<{ className?: string }>> = {
  github: Github,
  cloud: Cloud,
  search: Search,
  daytona: getCapabilityIcon("daytona"),
};

export function ProviderIcon({ iconName, className }: { iconName: string; className?: string }) {
  const Icon = iconMap[iconName] ?? LinkIcon;
  return <Icon className={className} />;
}
