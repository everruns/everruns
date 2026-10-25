"use client";

import { ChevronDown, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

interface ChatErrorAlertProps {
  message: string;
  description?: string;
  details?: string | null;
  /** False when resending cannot help (quota, keys, budgets); the card then says so. */
  retryable?: boolean;
  className?: string;
}

export function ChatErrorAlert({
  message,
  description,
  details,
  retryable = true,
  className,
}: ChatErrorAlertProps) {
  const { t } = useLocale();
  const displayMessage = message.trim() || t("chat_error_fallback");
  const trimmedDetails = details?.trim();
  const displayDescription =
    description?.trim() ||
    (retryable ? t("chat_error_description") : t("chat_error_action_description"));

  return (
    <div
      role="alert"
      className={cn(
        "animate-chat-row-in w-full rounded-lg border border-border/80 bg-card px-4 py-3 text-left",
        className,
      )}
    >
      <div className="flex items-center gap-2 text-destructive">
        <X className="h-4 w-4 flex-shrink-0" />
        <div className="text-sm font-semibold">{t("chat_error_title")}</div>
      </div>
      <p className="mt-2 text-sm leading-5 text-muted-foreground">{displayDescription}</p>
      <div className="mt-3 rounded-md bg-muted/70 px-3 py-2 font-mono text-sm leading-5 text-foreground">
        {displayMessage}
      </div>
      {trimmedDetails ? (
        <details className="group mt-2">
          <summary className="flex cursor-pointer list-none items-center gap-1 text-sm text-muted-foreground marker:hidden">
            <ChevronDown className="h-3.5 w-3.5 transition-transform group-open:rotate-180" />
            <span>{t("details")}</span>
          </summary>
          <pre className="mt-2 max-h-64 overflow-auto rounded-md bg-muted/60 px-3 py-2 text-xs leading-5 text-muted-foreground">
            {trimmedDetails}
          </pre>
        </details>
      ) : null}
    </div>
  );
}
