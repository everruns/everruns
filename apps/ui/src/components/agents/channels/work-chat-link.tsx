"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { CopyButton } from "@/components/ui/copy-button";
import type { AgentChannel } from "@/lib/api/types";

/// Hosted Work Chat addresses are `https://<channel>.<chat-domain>`. Anything
/// else is not a link we will open from the console.
export function workChatUrl(value: unknown): string | null {
  if (typeof value !== "string" || !value) return null;
  try {
    const url = new URL(value);
    const local = url.hostname === "localhost" || url.hostname === "127.0.0.1";
    if (url.protocol === "https:" || (url.protocol === "http:" && local)) return value;
  } catch {
    return null;
  }
  return null;
}

/// The address of a published Work Chat, from the channel's public bootstrap.
/// Absent until the channel is live and the deployment has a chat domain.
export function useWorkChatPublicUrl(channelId: string | null, live: boolean) {
  const hosted = useQuery({
    queryKey: ["hosted-chat-url", channelId],
    enabled: !!channelId && live,
    retry: false,
    queryFn: async () =>
      (await api.get<{ public_url?: string | null }>(`/v1/channels/${channelId}/chat/config`)).data,
  });
  return { url: workChatUrl(hosted.data?.public_url), loading: hosted.isLoading };
}

/// The address to send people, and a way to open it. Shared by the channel
/// page's settings and its Connect tab.
export function WorkChatAddress({
  channel,
  heading = true,
}: {
  channel: AgentChannel;
  heading?: boolean;
}) {
  const ui = useUiTranslations();
  const live = channel.enabled && channel.status === "live";
  const { url, loading } = useWorkChatPublicUrl(channel.id, live);
  return (
    <section className="flex flex-col gap-2" aria-label={heading ? undefined : ui("Chat link")}>
      {heading && <h3 className="text-sm font-semibold">{ui("Chat link")}</h3>}
      {url ? (
        <>
          <p className="text-sm text-muted-foreground">
            {ui(
              "Send this link to the people who should chat. It stays the same when you change settings.",
            )}
          </p>
          <div className="flex min-w-0 gap-2">
            <a
              href={url}
              target="_blank"
              rel="noopener noreferrer"
              className="flex h-8 min-w-0 flex-1 items-center overflow-x-auto border bg-muted/40 px-3 font-mono text-[13px] whitespace-nowrap hover:underline"
            >
              {url}
            </a>
            <CopyButton value={url} label={ui("Copy link")} className="my-0 size-8 border" />
          </div>
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          {loading
            ? ui("Loading…")
            : ui(
                "Publish this channel and configure your deployment’s hosted chat domain to open Work Chat.",
              )}
        </p>
      )}
    </section>
  );
}
