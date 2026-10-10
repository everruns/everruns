"use client";

import { AlertTriangle, X } from "lucide-react";
import { useConnectErrorFromUrl } from "@/lib/connect-error";

/** Shows a failed OAuth connect the server returned this page with. */
export function ConnectErrorBanner() {
  const [message, dismiss] = useConnectErrorFromUrl();
  if (!message) return null;
  return (
    <div
      role="alert"
      className="flex items-start gap-2 bg-destructive/10 p-3 text-sm text-destructive"
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
      <span className="flex-1">{message}</span>
      <button type="button" aria-label="Dismiss" onClick={dismiss} className="shrink-0">
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}
