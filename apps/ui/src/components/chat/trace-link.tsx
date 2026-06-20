"use client";

import { ExternalLink } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Small external-link affordance that opens a provider trace/logs page in a new
 * tab. Rendered next to message metadata and on turn dividers when the session's
 * provider has trace links enabled (see ProviderTraceConfig).
 */
export function TraceLink({
  href,
  label,
  variant = "default",
  showLabel = false,
  className,
}: {
  href: string;
  label: string;
  variant?: "default" | "light";
  showLabel?: boolean;
  className?: string;
}) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(e) => e.stopPropagation()}
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex flex-shrink-0 items-center gap-1 rounded p-0.5 transition-colors",
        variant === "light"
          ? "text-white/55 hover:bg-white/12 hover:text-white"
          : "text-muted-foreground/55 hover:bg-muted hover:text-foreground",
        className,
      )}
    >
      <ExternalLink className="h-3 w-3" />
      {showLabel && <span className="whitespace-nowrap">{label}</span>}
    </a>
  );
}
