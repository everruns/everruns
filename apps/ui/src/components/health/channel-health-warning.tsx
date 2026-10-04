"use client";

import Link from "next/link";
import { AlertTriangle } from "lucide-react";
import { useHealthIssues } from "@/hooks/use-health-issues";

export function ChannelHealthWarning({ channelId }: { channelId: string }) {
  const issues = useHealthIssues(channelId);
  if (issues.isError)
    return (
      <p role="status" className="text-xs text-muted-foreground">
        Could not load integration health.
      </p>
    );
  return (issues.data?.data ?? []).map((issue) => (
    <div key={issue.id} className="flex items-start gap-2 border border-border p-3">
      <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" />
      <div className="min-w-0 space-y-1">
        <p className="text-sm font-medium">{issue.title}</p>
        <p className="text-xs text-muted-foreground">{issue.body}</p>
        <Link href={issue.href} className="text-xs underline underline-offset-2">
          Review issue
        </Link>
      </div>
    </div>
  ));
}
