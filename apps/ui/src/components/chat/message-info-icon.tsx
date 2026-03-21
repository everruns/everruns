"use client";

import { Info } from "lucide-react";
import { Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/tooltip";
import { CopyButton } from "@/components/ui/copy-button";
import { cn } from "@/lib/utils";
import type { Event, TokenUsage } from "@/lib/api/types";
import { getEventData, isRecord } from "@/lib/api/types";
import { useLocale } from "@/providers/locale-provider";

interface MessageInfoIconProps {
  /** The event containing message data */
  event: Event;
  /** Visual variant - "default" for dark text contexts, "light" for light text contexts (e.g., user messages) */
  variant?: "default" | "light";
}

/**
 * Small info icon that displays message metadata in a tooltip.
 * Shows: message ID, model, reasoning effort, timestamp, and token usage.
 */
export function MessageInfoIcon({ event, variant = "default" }: MessageInfoIconProps) {
  const { t } = useLocale();
  const agentData = getEventData(event, "output.message.completed");

  // Model and reasoning are stored on message.metadata (set by ReasonAtom)
  // Fallback to agentData.metadata for backward compatibility
  const messageMetadata = isRecord(agentData?.message?.metadata)
    ? agentData.message.metadata
    : undefined;
  const model =
    (typeof messageMetadata?.model === "string" ? messageMetadata.model : undefined) ??
    agentData?.metadata?.model;
  const reasoningEffort =
    typeof messageMetadata?.reasoning_effort === "string"
      ? messageMetadata.reasoning_effort
      : undefined;
  const phase = agentData?.message?.phase;

  // Token usage from OutputMessageCompletedData (if populated)
  const usage: TokenUsage | undefined = agentData?.usage;

  // Format timestamp
  const timestamp = new Date(event.ts);
  const formattedTime = timestamp.toLocaleString();

  return (
    <Tooltip>
      <TooltipTrigger
        className={cn(
          "flex-shrink-0 rounded p-0.5 transition-colors",
          variant === "light"
            ? "text-white/55 hover:bg-white/12 hover:text-white"
            : "text-muted-foreground/55 hover:bg-muted hover:text-foreground",
        )}
        aria-label={t("message_info")}
      >
        <Info className="w-3 h-3" />
      </TooltipTrigger>
      <TooltipContent className="max-w-sm border-border bg-popover px-3 py-2">
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-muted-foreground">{t("message_info_id")}</dt>
          <dd>
            <CopyButton value={event.id} />
          </dd>
          {model && (
            <>
              <dt className="text-muted-foreground">{t("message_info_model")}</dt>
              <dd>{model}</dd>
            </>
          )}
          {reasoningEffort && (
            <>
              <dt className="text-muted-foreground">{t("message_info_reasoning")}</dt>
              <dd className="capitalize">{reasoningEffort}</dd>
            </>
          )}
          {phase && (
            <>
              <dt className="text-muted-foreground">{t("message_info_phase")}</dt>
              <dd className="capitalize">{phase.replace("_", " ")}</dd>
            </>
          )}
          <dt className="text-muted-foreground">{t("message_info_time")}</dt>
          <dd>{formattedTime}</dd>
          {usage && (
            <>
              <dt className="text-muted-foreground">{t("message_info_tokens")}</dt>
              <dd>
                {t("message_info_tokens_value", {
                  input: usage.input_tokens,
                  output: usage.output_tokens,
                })}
              </dd>
            </>
          )}
        </dl>
      </TooltipContent>
    </Tooltip>
  );
}
