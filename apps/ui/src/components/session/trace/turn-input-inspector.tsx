"use client";

import { useQuery } from "@tanstack/react-query";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import { getSessionTraceTurnInput } from "@/lib/api/session-trace";
import { useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";
import { formatCount } from "./trace-model";

export function TurnInputInspector({
  sessionId,
  turn,
  onClose,
}: {
  sessionId: string;
  turn: number;
  onClose: () => void;
}) {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const {
    data: input,
    isLoading,
    isError,
  } = useQuery({
    queryKey: ["session-trace", currentOrg?.public_id, sessionId, "turn-input", turn],
    queryFn: async () => (await getSessionTraceTurnInput(sessionId, turn)).content,
  });
  return (
    <section>
      <div className="top-0 z-10 flex items-center gap-2 border-b bg-card px-5 py-3 lg:sticky">
        <h2 className="min-w-0 flex-1 text-sm font-semibold">{ui("Turn input")}</h2>
        <span className="text-xs text-muted-foreground">{formatCount(turn)}</span>
        {input && <CopyButton value={input} label={ui("Copy")} />}
        <Button
          size="icon-sm"
          variant="ghost"
          className="hidden lg:inline-flex"
          aria-label={ui("Close")}
          onClick={onClose}
        >
          <X />
        </Button>
      </div>
      <div className="px-5 py-4">
        {isLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : isError ? (
          <p role="alert" className="text-sm text-destructive">
            {ui("Could not load the message.")}
          </p>
        ) : input ? (
          <pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed">
            {input}
          </pre>
        ) : (
          <p className="text-sm text-muted-foreground">{ui("No prompt")}</p>
        )}
      </div>
    </section>
  );
}
