"use client";

/**
 * Inline consent card for a URL an MCP server asked the user to open.
 *
 * An MCP server that needs a secret, an authorization, or a payment answers the
 * tool call with a URL instead of asking through the client — the value is typed
 * into the user's own browser, so it never reaches the model or the event log.
 * The backend pauses the turn and emits a synthetic `confirm_url_elicitation`
 * call; this card is how a person decides.
 *
 * Three rules from the elicitation spec shape the markup:
 *
 * - The full URL is shown, never a bare "click here". The domain is highlighted
 *   because that is the part worth reading.
 * - A Punycode domain is flagged. Internationalized domains are legitimate, but
 *   the user should know before trusting one.
 * - Nothing opens without an explicit click, and the page is never prefetched.
 *
 * Consent is collected in two steps — open, then confirm you are done — because
 * the client cannot know when an out-of-band interaction finished. Answering
 * "accept" the moment the tab opens resumes the turn too early: the server
 * checks, finds nothing done yet, and elicits all over again.
 */

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { AlertTriangle, Check, ExternalLink, X } from "lucide-react";
import { submitElicitationConsent } from "@/lib/api/sessions";
import { cn } from "@/lib/utils";
import type { ToolCompletedData } from "@/lib/api/types";

export interface UrlElicitationArguments {
  server?: string;
  tool?: string;
  message?: string;
  url?: string;
  url_host?: string;
  url_is_punycode?: boolean;
}

interface UrlElicitationToolCallProps {
  sessionId: string;
  toolCallId: string;
  elicitation: UrlElicitationArguments;
  /** Existing tool results map — if a result already exists, show completed state */
  toolResultsMap: Map<string, ToolCompletedData>;
}

/**
 * Split the URL around its host so the domain can be emphasised in place,
 * keeping the rest of the URL visible rather than truncated away.
 */
function splitOnHost(url: string, host: string): [string, string, string] {
  const index = host ? url.indexOf(host) : -1;
  if (index < 0) return [url, "", ""];
  return [url.slice(0, index), host, url.slice(index + host.length)];
}

export function UrlElicitationToolCall({
  sessionId,
  toolCallId,
  elicitation,
  toolResultsMap,
}: UrlElicitationToolCallProps) {
  const [status, setStatus] = useState<"idle" | "submitting" | "accepted" | "declined">("idle");
  // Opening the link and finishing what is on the other side are two different
  // moments, and only the person knows when the second one arrives.
  const [opened, setOpened] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const url = elicitation.url ?? "";
  const host = elicitation.url_host ?? "";
  const server = elicitation.server ?? "An MCP server";
  const [before, domain, after] = splitOnHost(url, host);

  const existingResult = toolResultsMap.get(toolCallId);
  const isCompleted = existingResult != null || status === "accepted" || status === "declined";

  const decide = async (action: "accept" | "decline") => {
    setStatus("submitting");
    setError(null);
    try {
      await submitElicitationConsent(sessionId, toolCallId, action);
      setStatus(action === "accept" ? "accepted" : "declined");
    } catch {
      setStatus("idle");
      setError("Could not record your answer. Try again.");
    }
  };

  const handleOpen = () => {
    // Opened from the click itself, so the browser treats it as user-initiated
    // and `noopener` keeps the opened page away from this one.
    window.open(url, "_blank", "noopener,noreferrer");
    setOpened(true);
  };

  if (isCompleted) {
    const accepted = status === "accepted" || (existingResult?.success ?? false);
    return (
      <div
        className={cn(
          "flex items-center gap-2 px-3 py-1.5 text-sm",
          accepted ? "text-green-700 dark:text-green-400" : "text-muted-foreground",
        )}
      >
        {accepted ? <Check className="h-4 w-4" /> : <X className="h-4 w-4" />}
        {accepted ? `Continued at ${host}` : `Declined to open ${host}`}
      </div>
    );
  }

  return (
    <div className="border border-border bg-muted/50 px-4 py-3">
      <div className="flex items-start gap-3">
        <ExternalLink className="mt-0.5 h-5 w-5 shrink-0 text-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">
            {server} needs you to finish something in your browser
          </p>
          {elicitation.message && (
            <p className="mt-0.5 text-xs text-muted-foreground">{elicitation.message}</p>
          )}
          {/* The full URL, with the domain emphasised — the part that decides
              whether this is safe to open. */}
          <p className="mt-2 break-all font-mono text-xs text-muted-foreground">
            {before}
            <span className="font-semibold text-foreground">{domain}</span>
            {after}
          </p>
          {elicitation.url_is_punycode && (
            <p className="mt-2 flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-400">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>
                This domain is written in Punycode, so it can look like a different one. Check it
                before continuing.
              </span>
            </p>
          )}
          <p className="mt-2 text-xs text-muted-foreground">
            {opened
              ? "Finish on that page, then come back and continue. Everruns never sees what you enter there."
              : "Everruns never sees what you enter there. Come back here when you are done."}
          </p>
          {error && <p className="mt-2 text-xs text-destructive">{error}</p>}
        </div>
      </div>
      <div className="mt-3 flex items-center justify-end gap-2">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void decide("decline")}
          disabled={status === "submitting"}
          className="text-muted-foreground"
        >
          {opened ? "Cancel" : "Don't open"}
        </Button>
        {opened ? (
          <Button
            size="sm"
            onClick={() => void decide("accept")}
            disabled={status === "submitting"}
          >
            <Check className="mr-1 h-3.5 w-3.5" />
            I&apos;ve finished — continue
          </Button>
        ) : (
          <Button size="sm" onClick={handleOpen} disabled={status === "submitting" || !url}>
            <ExternalLink className="mr-1 h-3.5 w-3.5" />
            Open link
          </Button>
        )}
      </div>
    </div>
  );
}
