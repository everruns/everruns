"use client";

import { Info } from "lucide-react";
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { Event, MessageAgentData, ModelMetadata, TokenUsage } from "@/lib/api/types";

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
  const isAgentMessage = event.type === "message.agent";
  const agentData = isAgentMessage ? (event.data as MessageAgentData) : null;
  const modelMetadata: ModelMetadata | undefined = agentData?.metadata;
  const usage: TokenUsage | undefined = agentData?.usage;

  // Extract reasoning effort from event metadata if available
  const reasoningEffort = (event.metadata as Record<string, unknown> | undefined)?.reasoning_effort as string | undefined;

  // Format timestamp
  const timestamp = new Date(event.ts);
  const formattedTime = timestamp.toLocaleString();

  return (
    <Tooltip>
      <TooltipTrigger
        className={cn(
          "p-0.5 rounded transition-colors flex-shrink-0",
          variant === "light"
            ? "text-white/50 hover:text-white hover:bg-white/10"
            : "text-muted-foreground/50 hover:text-muted-foreground hover:bg-muted/80"
        )}
        aria-label="Message info"
      >
        <Info className="w-3 h-3" />
      </TooltipTrigger>
      <TooltipContent className="max-w-sm">
        <div className="space-y-1 text-xs">
          <div className="flex gap-2">
            <span className="text-muted-foreground">ID:</span>
            <span className="font-mono text-[10px] break-all">{event.id}</span>
          </div>
          {modelMetadata?.model && (
            <div className="flex gap-2">
              <span className="text-muted-foreground">Model:</span>
              <span>{modelMetadata.model}</span>
            </div>
          )}
          {reasoningEffort && (
            <div className="flex gap-2">
              <span className="text-muted-foreground">Reasoning:</span>
              <span className="capitalize">{reasoningEffort}</span>
            </div>
          )}
          <div className="flex gap-2">
            <span className="text-muted-foreground">Time:</span>
            <span>{formattedTime}</span>
          </div>
          {usage && (
            <div className="flex gap-2">
              <span className="text-muted-foreground">Tokens:</span>
              <span>{usage.input_tokens} in / {usage.output_tokens} out</span>
            </div>
          )}
        </div>
      </TooltipContent>
    </Tooltip>
  );
}
