"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { ChevronRight, Copy, ExternalLink, MoreHorizontal } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EntityStatus, type EntityStatusTone } from "@/components/ui/entity-status";
import type { AgentChannel, ChannelType } from "@/lib/api/types";
import { ChannelIcon } from "./channel-icon";
import {
  CHANNEL_STATUS_LABEL,
  channelDisplayName,
  channelFacts,
  channelStatus,
  lastUsedLabel,
  publicChatUrl,
  type ChannelStatus,
} from "./channel-summary";

const STATUS_TONE: Record<ChannelStatus, EntityStatusTone> = {
  published: "success",
  draft: "muted",
  needs_setup: "warning",
};

/// The channels on the agent's Integrations tab: one row each, the whole row
/// opens the channel page. Configuration, how to connect, and budget live on
/// that page, not in an expanding row.
export function ChannelList({
  channels,
  hrefFor,
  onPublishChange,
  publishPending = false,
}: {
  channels: AgentChannel[];
  hrefFor: (channel: AgentChannel) => string;
  onPublishChange?: (channel: AgentChannel, publish: boolean) => void;
  publishPending?: boolean;
}) {
  const ui = useUiTranslations();
  return (
    <div className="@container/channels border bg-card" role="table" aria-label={ui("Channels")}>
      <div
        role="row"
        className="hidden grid-cols-[2rem_minmax(0,1fr)_7.5rem_8.5rem_4.5rem] gap-4 border-b bg-muted/30 px-4 py-2 text-xs font-medium text-muted-foreground @min-[40rem]/channels:grid"
      >
        <span role="columnheader" className="col-start-2">
          {ui("Channel")}
        </span>
        <span role="columnheader">{ui("Status")}</span>
        <span role="columnheader">{ui("Last used")}</span>
      </div>
      {channels.map((channel) => (
        <ChannelListRow
          key={channel.id}
          channel={channel}
          href={hrefFor(channel)}
          onPublishChange={onPublishChange}
          publishPending={publishPending}
        />
      ))}
    </div>
  );
}

function ChannelListRow({
  channel,
  href,
  onPublishChange,
  publishPending,
}: {
  channel: AgentChannel;
  href: string;
  onPublishChange?: (channel: AgentChannel, publish: boolean) => void;
  publishPending: boolean;
}) {
  const ui = useUiTranslations();
  const status = channelStatus(channel);
  const name = channelDisplayName(channel);
  const live = channel.enabled && channel.status === "live";
  const link = channel.channel_type === "public_chat" ? publicChatUrl(channel.id) : null;
  const hasMenu = !!link || !!onPublishChange;
  return (
    <div
      role="row"
      className="relative grid grid-cols-[2rem_minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 border-border/60 px-4 py-3.5 text-sm transition-colors hover:bg-muted/50 has-[a:focus-visible]:bg-muted/50 [&+&]:border-t @min-[40rem]/channels:grid-cols-[2rem_minmax(0,1fr)_7.5rem_8.5rem_4.5rem]"
    >
      <span className="flex size-8 items-center justify-center border bg-background" role="cell">
        <ChannelIcon kind={channel.channel_type as ChannelType} className="icon-sharp size-4" />
      </span>
      <div className="min-w-0" role="cell">
        {/* The name is the row's link; its overlay makes the whole row clickable. */}
        <Link
          href={href}
          className="block truncate font-semibold outline-none after:absolute after:inset-0"
        >
          {name}
        </Link>
        <p className="mt-0.5 truncate text-muted-foreground">
          {channelFacts(channel)
            // A channel with no name of its own is named by its type already.
            .filter((fact) => fact !== name)
            .join(" · ")}
        </p>
        <div className="mt-1 flex items-center gap-3 @min-[40rem]/channels:hidden">
          <EntityStatus
            status={status}
            tone={STATUS_TONE[status]}
            label={CHANNEL_STATUS_LABEL[status]}
          />
          <span className="text-xs text-muted-foreground">
            {lastUsedLabel(channel.last_invoked_at)}
          </span>
        </div>
      </div>
      <span role="cell" className="hidden @min-[40rem]/channels:block">
        <EntityStatus
          status={status}
          tone={STATUS_TONE[status]}
          label={CHANNEL_STATUS_LABEL[status]}
        />
      </span>
      <span
        role="cell"
        className="hidden text-muted-foreground tabular-nums @min-[40rem]/channels:block"
      >
        {lastUsedLabel(channel.last_invoked_at)}
      </span>
      <span
        role="cell"
        className="relative z-10 flex items-center justify-end gap-1 text-muted-foreground"
      >
        {hasMenu && (
          <DropdownMenu>
            <DropdownMenuTrigger
              className={buttonVariants({ variant: "ghost", size: "icon-sm" })}
              aria-label={ui("Actions for {value0}", { value0: name })}
            >
              <MoreHorizontal className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner align="end">
              <DropdownMenuContent>
                {link && (
                  <>
                    <DropdownMenuItem onClick={() => void navigator.clipboard?.writeText(link)}>
                      <Copy className="size-4" />
                      {ui("Copy link")}
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => window.open(link, "_blank", "noopener,noreferrer")}
                    >
                      <ExternalLink className="size-4" />
                      {ui("Open chat page")}
                    </DropdownMenuItem>
                  </>
                )}
                {link && onPublishChange && <DropdownMenuSeparator />}
                {onPublishChange && (
                  <DropdownMenuItem
                    disabled={publishPending}
                    onClick={() => onPublishChange(channel, !live)}
                  >
                    {live ? ui("Unpublish") : ui("Publish")}
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        )}
        <ChevronRight aria-hidden="true" className="size-4" />
      </span>
    </div>
  );
}
