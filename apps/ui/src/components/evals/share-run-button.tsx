"use client";

// Mint / copy / revoke a read-only share link for an eval run. The raw token is
// returned once by the server (stored only hashed), so the link is shown right
// after minting; afterward we only know whether a link is active, not its value.

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createRunShare, revokeRunShare, getRunShareStatus } from "@/lib/api/evals";
import { Button } from "@/components/ui/button";
import { Share2, Copy, Check } from "lucide-react";

export function ShareRunButton({ evalId, runId }: { evalId: string; runId: string }) {
  const queryClient = useQueryClient();
  const [link, setLink] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const shareKey = ["eval-run-share", evalId, runId];
  const status = useQuery({
    queryKey: shareKey,
    queryFn: () => getRunShareStatus(evalId, runId),
  });

  const mint = useMutation({
    mutationFn: () => createRunShare(evalId, runId),
    onSuccess: (data) => {
      setLink(`${window.location.origin}/shared/eval-runs/${data.token}`);
      queryClient.invalidateQueries({ queryKey: shareKey });
    },
  });

  const revoke = useMutation({
    mutationFn: () => revokeRunShare(evalId, runId),
    onSuccess: () => {
      setLink(null);
      queryClient.invalidateQueries({ queryKey: shareKey });
    },
  });

  const active = status.data?.active ?? false;

  const copy = async () => {
    if (!link) return;
    await navigator.clipboard?.writeText(link);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="flex flex-col items-end gap-2">
      <div className="flex gap-2">
        <Button variant="outline" size="sm" onClick={() => mint.mutate()} disabled={mint.isPending}>
          <Share2 className="w-4 h-4 mr-2" />
          {active || link ? "Regenerate link" : "Share"}
        </Button>
        {(active || link) && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => revoke.mutate()}
            disabled={revoke.isPending}
          >
            Revoke
          </Button>
        )}
      </div>
      {link && (
        <div className="flex items-center gap-1">
          <input
            readOnly
            value={link}
            aria-label="Public share link"
            onFocus={(e) => e.currentTarget.select()}
            className="w-72 rounded border bg-muted px-2 py-1 text-xs"
          />
          <Button variant="ghost" size="icon" onClick={copy} aria-label="Copy share link">
            {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
          </Button>
        </div>
      )}
      {link && (
        <p className="text-[11px] text-muted-foreground">
          Anyone with this link can view this run read-only.
        </p>
      )}
    </div>
  );
}
