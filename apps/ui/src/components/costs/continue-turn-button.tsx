"use client";

// The answer to a usage policy's "ask to continue" step: lets this session
// past the step for the policy's current window, then asks the agent to pick
// up where it stopped. Stop steps still stop.

import { useState } from "react";
import { useMutation } from "@tanstack/react-query";

import { Button } from "@/components/ui/button";
import { continueSessionPastAsk } from "@/lib/api/costs";
import { sendUserMessage } from "@/lib/api/messages";
import { useUiTranslations } from "@/providers/locale-provider";

export function ContinueTurnButton({ sessionId }: { sessionId: string }) {
  const ui = useUiTranslations();
  const [done, setDone] = useState(false);
  const proceed = useMutation({
    mutationFn: async () => {
      await continueSessionPastAsk(sessionId);
      await sendUserMessage(sessionId, ui("Continue where you stopped."));
    },
    onSuccess: () => setDone(true),
  });
  if (done) {
    return <p className="mt-3 text-sm text-muted-foreground">{ui("Continuing this session.")}</p>;
  }
  return (
    <div className="mt-3 flex flex-wrap items-center gap-3">
      <Button size="sm" onClick={() => proceed.mutate()} disabled={proceed.isPending}>
        {ui("Continue this turn")}
      </Button>
      <span className="text-xs text-muted-foreground">
        {ui("Applies to this session until the policy's window resets.")}
      </span>
      {proceed.error ? (
        <span role="alert" className="text-xs text-destructive">
          {proceed.error.message}
        </span>
      ) : null}
    </div>
  );
}
