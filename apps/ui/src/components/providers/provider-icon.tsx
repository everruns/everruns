import Image from "next/image";
import { Server } from "lucide-react";
import type { LlmProviderType } from "@/lib/api/types";
import { cn } from "@/lib/utils";

const PROVIDER_ICONS: Record<LlmProviderType, string> = {
  openai: "/providers/openai.svg",
  anthropic: "/providers/anthropic.svg",
  azure_openai: "/providers/azure.svg",
};

const PROVIDER_LABELS: Record<LlmProviderType, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  azure_openai: "Azure OpenAI",
};

interface ProviderIconProps {
  providerType: LlmProviderType;
  size?: "sm" | "md" | "lg";
  className?: string;
  showBackground?: boolean;
}

const sizeMap = {
  sm: { icon: 16, container: "p-1.5" },
  md: { icon: 20, container: "p-2" },
  lg: { icon: 24, container: "p-2.5" },
};

export function ProviderIcon({
  providerType,
  size = "md",
  className,
  showBackground = true,
}: ProviderIconProps) {
  const iconPath = PROVIDER_ICONS[providerType];
  const label = PROVIDER_LABELS[providerType];
  const { icon: iconSize, container } = sizeMap[size];

  if (!iconPath) {
    return (
      <div
        className={cn(
          showBackground && "bg-primary/10 rounded-lg",
          container,
          className
        )}
      >
        <Server
          className="text-primary"
          style={{ width: iconSize, height: iconSize }}
        />
      </div>
    );
  }

  return (
    <div
      className={cn(
        showBackground && "bg-primary/10 rounded-lg",
        container,
        className
      )}
      title={label}
    >
      <Image
        src={iconPath}
        alt={label}
        width={iconSize}
        height={iconSize}
        className="dark:invert"
      />
    </div>
  );
}

export function getProviderLabel(providerType: LlmProviderType): string {
  return PROVIDER_LABELS[providerType] || providerType;
}
