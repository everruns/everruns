"use client";

import { AlertCircle, ChevronDown, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { CopyButton } from "@/components/ui/copy-button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ApiError } from "@/lib/api/client";

export function SlackInstallError({ error, onRetry }: { error: Error; onRetry: () => void }) {
  const apiError = error instanceof ApiError ? error : undefined;
  const details = apiError
    ? [
        `HTTP ${apiError.status}${apiError.statusText ? ` · ${apiError.statusText}` : ""}`,
        apiError.message,
        apiError.code && `Error code: ${apiError.code}`,
        apiError.requestId && `Request ID: ${apiError.requestId}`,
        apiError.rayId && `Gateway ID: ${apiError.rayId}`,
      ]
        .filter(Boolean)
        .join("\n")
    : "The request could not be completed. Check your connection and try again.";
  const description =
    apiError && apiError.status < 500
      ? apiError.message
      : "The connection was interrupted. Try again to continue setup.";

  return (
    <Notice variant="destructive" role="alert" icon={<AlertCircle className="size-4" />}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <NoticeTitle className="text-sm">Couldn’t connect to Slack</NoticeTitle>
          <NoticeDescription className="text-xs">{description}</NoticeDescription>
        </div>
        <Button type="button" variant="outline" size="sm" onClick={onRetry}>
          <RefreshCw className="size-3.5" />
          Try again
        </Button>
      </div>
      <Collapsible className="mt-3 border-t pt-2">
        <CollapsibleTrigger className="group flex items-center gap-1 text-xs text-muted-foreground">
          <ChevronDown className="size-3.5 -rotate-90 transition-transform group-data-[panel-open]:rotate-0" />
          Technical details
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="mt-2 flex items-start gap-2 bg-muted p-3">
            <pre className="max-h-40 min-w-0 flex-1 overflow-auto whitespace-pre-wrap break-words font-mono text-xs text-muted-foreground">
              {details}
            </pre>
            <CopyButton value={details} label="Copy technical details" />
          </div>
        </CollapsibleContent>
      </Collapsible>
    </Notice>
  );
}
