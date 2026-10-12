"use client";

// Prompt of one model call split the way providers bill it: read from the
// prompt cache, written to it, and uncached. One stacked bar plus a legend,
// and a note when the call lost the previous call's cache or ran a long prompt
// with caching off.

import type { TraceRequestUsage } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { formatCount } from "./trace-model";

// Why a cache reset happened, in the words a reader acts on.
const RESET_REASONS: Record<string, string> = {
  model_changed: "the model changed since the previous call",
  tools_changed: "the tool definitions changed since the previous call",
  system_changed: "the system prompt changed since the previous call",
  history_changed: "earlier messages were rewritten, for example by compaction",
  expired: "the cache expired while the session was idle",
  unknown: "the provider did not say why",
};

const SEGMENTS = [
  { key: "cache_read_tokens", label: "Cache read", swatch: "bg-success" },
  { key: "cache_creation_tokens", label: "Cache write", swatch: "bg-info" },
  { key: "input_tokens", label: "Uncached", swatch: "bg-muted-foreground/45" },
] as const;

export function CacheBar({ usage, className }: { usage: TraceRequestUsage; className?: string }) {
  const prompt = usage.input_tokens + usage.cache_read_tokens + usage.cache_creation_tokens;
  if (prompt <= 0) return null;
  const hitRate = Math.round((usage.cache_read_tokens / prompt) * 100);
  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-baseline gap-2 text-xs">
        <span className="font-medium">{formatCount(prompt)} prompt tokens</span>
        <span className="text-muted-foreground">
          {hitRate}% from cache · {formatCount(usage.output_tokens)} out
          {usage.prompt_cache ? ` · caching: ${usage.prompt_cache}` : " · caching off"}
        </span>
      </div>
      <div
        className="flex h-2 w-full overflow-hidden bg-muted"
        role="img"
        aria-label={`${hitRate}% of the prompt read from cache`}
      >
        {SEGMENTS.map(({ key, swatch, label }) =>
          usage[key] > 0 ? (
            <div
              key={key}
              className={swatch}
              style={{ width: `${(usage[key] / prompt) * 100}%` }}
              title={`${label}: ${formatCount(usage[key])}`}
            />
          ) : null,
        )}
      </div>
      <ul className="flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
        {SEGMENTS.map(({ key, swatch, label }) => (
          <li key={key} className="flex items-center gap-1">
            <span className={cn("inline-block size-2", swatch)} />
            {label} <span className="font-mono text-foreground">{formatCount(usage[key])}</span>
          </li>
        ))}
      </ul>
      {usage.cache_reset && (
        <p className="text-[11px] text-warning" role="note">
          Cache reset: {formatCount(usage.cache_reset.lost_tokens)} cached tokens not reused because{" "}
          {RESET_REASONS[usage.cache_reset.reason] ?? RESET_REASONS.unknown}
          {usage.cache_reset.reason === "expired" &&
            ` (idle ${formatIdle(usage.cache_reset.idle_seconds)})`}
          .
        </p>
      )}
      {usage.caching_would_help && (
        <p className="text-[11px] text-muted-foreground" role="note">
          This long prompt was sent with caching off. The Prompt Caching capability would let later
          calls reuse it.
        </p>
      )}
    </div>
  );
}

function formatIdle(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  return `${(seconds / 3600).toFixed(1)}h`;
}
