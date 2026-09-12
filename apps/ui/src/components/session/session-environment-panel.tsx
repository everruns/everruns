"use client";

// Environment — where this session's commands run, and what they may touch.
//
// It sits on the Workspace tab because the files above it and the compute here
// are two halves of one thing: the same working filesystem, addressed by the
// file tools and by `bash`.
//
// The capability rows are the point. Bashkit runs no native binaries, so a
// build cannot succeed there; showing that as a plain row means an operator
// reads it before a run fails rather than after.

import { useQuery } from "@tanstack/react-query";
import { Box, Check, Cpu, Minus, ShieldCheck, ShieldOff } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api/client";
import type { SessionEnvironment } from "@/lib/api/environments";

const TARGET_LABELS: Record<string, string> = {
  host: "This machine",
  machine: "Registered machine",
  vfs: "Virtual filesystem",
  container: "Container",
  managed: "Managed sandbox",
};

const CONTAINMENT_LABELS: Record<string, string> = {
  none: "Uncontained",
  native: "Kernel policy",
  isolated: "Isolated",
};

const DURABILITY_LABELS: Record<string, string> = {
  checkpointed: "Checkpointed",
  provider_snapshot: "Provider snapshot only",
  none: "Not recoverable",
};

const CAPABILITY_ROWS: Array<{ key: keyof SessionEnvironment["capabilities"]; label: string }> = [
  { key: "native_processes", label: "Native processes" },
  { key: "packages", label: "Package installs" },
  { key: "pty", label: "Interactive terminal" },
  { key: "ports", label: "Listening ports" },
  { key: "portable_checkpoint", label: "Portable checkpoint" },
  { key: "network_enforced", label: "Network policy enforced" },
];

function useSessionEnvironment(sessionId: string) {
  return useQuery({
    queryKey: ["session-environment", sessionId],
    queryFn: async () => {
      const response = await api.get<SessionEnvironment>(`/v1/sessions/${sessionId}/environment`);
      return response.data;
    },
  });
}

function CapabilityRow({ label, enabled }: { label: string; enabled: boolean }) {
  return (
    <div className="flex items-center justify-between gap-3 py-1">
      <span className={enabled ? "text-sm" : "text-sm text-muted-foreground"}>{label}</span>
      {enabled ? (
        <span className="flex items-center gap-1 text-xs text-foreground">
          <Check className="h-3 w-3" aria-hidden />
          yes
        </span>
      ) : (
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          <Minus className="h-3 w-3" aria-hidden />
          no
        </span>
      )}
    </div>
  );
}

export function SessionEnvironmentPanel({ sessionId }: { sessionId: string }) {
  const { data, isLoading, isError } = useSessionEnvironment(sessionId);

  if (isLoading) {
    return (
      <div className="border-t p-4">
        <h2 className="mb-3 text-sm font-medium text-muted-foreground">Environment</h2>
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }

  if (isError || !data) {
    return null;
  }

  const targetLabel = data.target
    ? (TARGET_LABELS[data.target.kind] ?? data.target.kind)
    : "No compute";
  const contained = data.containment.level !== "none";

  return (
    <div className="border-t p-4">
      <h2 className="mb-3 text-sm font-medium text-muted-foreground">Environment</h2>

      <div className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline" className="flex items-center gap-1.5">
            {data.target ? (
              <Cpu className="h-3 w-3" aria-hidden />
            ) : (
              <Box className="h-3 w-3" aria-hidden />
            )}
            {targetLabel}
          </Badge>
          {data.target?.provider ? (
            <Badge variant="secondary" className="font-mono">
              {data.target.provider}
            </Badge>
          ) : null}
          <Badge
            variant={contained ? "outline" : "destructive"}
            className="flex items-center gap-1.5"
          >
            {contained ? (
              <ShieldCheck className="h-3 w-3" aria-hidden />
            ) : (
              <ShieldOff className="h-3 w-3" aria-hidden />
            )}
            {CONTAINMENT_LABELS[data.containment.level] ?? data.containment.level}
          </Badge>
          <Badge variant="outline">network {data.containment.network}</Badge>
        </div>

        {data.target ? (
          <div className="border p-3">
            <div className="mb-2 text-xs uppercase tracking-[0.14em] text-muted-foreground">
              What it can do
            </div>
            <div className="divide-y">
              {CAPABILITY_ROWS.map((row) => (
                <CapabilityRow
                  key={row.key}
                  label={row.label}
                  enabled={data.capabilities[row.key]}
                />
              ))}
            </div>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            This session runs no commands. It reads and writes files only.
          </p>
        )}

        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span>Recovery: {DURABILITY_LABELS[data.durability] ?? data.durability}</span>
          {data.source_capability ? (
            <span className="font-mono">from {data.source_capability}</span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
