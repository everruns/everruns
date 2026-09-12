"use client";

// Inline verdict for {@link useCredentialCheck}, rendered next to the credential
// input itself so the result reads as a property of what was typed rather than
// as a separate step. `inconclusive` renders nothing: it says nothing about the
// credential, and an "unknown" chip would only invite a pointless retry.

import { Check, Loader2, TriangleAlert } from "lucide-react";
import type { CredentialCheckState } from "./use-credential-check";

export function CredentialCheckStatus({
  state,
  className,
}: {
  state: CredentialCheckState;
  className?: string;
}) {
  if (state.status === "checking") {
    return (
      <span
        className={`flex items-center gap-1.5 text-xs text-muted-foreground ${className ?? ""}`}
        role="status"
      >
        <Loader2 className="icon-sharp size-3.5 animate-spin" />
        Checking key…
      </span>
    );
  }

  if (state.status === "valid") {
    return (
      <span
        className={`flex items-center gap-1.5 text-xs font-medium text-emerald-700 dark:text-emerald-400 ${className ?? ""}`}
        role="status"
      >
        <Check className="icon-sharp size-3.5" />
        {state.models > 0 ? `Key valid — ${state.models} models` : "Key valid"}
      </span>
    );
  }

  if (state.status === "rejected") {
    return (
      <span
        className={`flex items-center gap-1.5 text-xs font-medium text-destructive ${className ?? ""}`}
        role="status"
      >
        <TriangleAlert className="icon-sharp size-3.5" />
        Key rejected
      </span>
    );
  }

  return null;
}
