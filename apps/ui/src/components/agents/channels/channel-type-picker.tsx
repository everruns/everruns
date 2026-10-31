"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ChannelType } from "@/lib/api/types";
import { getChannelTypeDisplayName } from "@/lib/channel-display";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { CHANNEL_FORM_KINDS } from "./channel-form";
import { ChannelIcon } from "./channel-icon";
import { CHANNEL_TYPE_INFO, channelTypeInfo, type ChannelAudience } from "./channel-types";

/// The channel types a person can add from the New channel page, in two
/// groups by who uses them. Feature flags still hide Public Chat and Voice.
function useChoosableChannelTypes() {
  const publicChatEnabled = useFeatureFlag("public_chat");
  const voiceEnabled = useFeatureFlag("voice");
  return CHANNEL_TYPE_INFO.filter(
    ({ kind }) =>
      CHANNEL_FORM_KINDS.includes(kind) &&
      (kind !== "public_chat" || publicChatEnabled) &&
      (kind !== "voice" || voiceEnabled),
  );
}

const AUDIENCES: { audience: ChannelAudience; title: string; hint: string }[] = [
  { audience: "people", title: "For people", hint: "Someone types or talks to the agent" },
  {
    audience: "software",
    title: "For software",
    hint: "Your code or another system calls the agent",
  },
];

/// Rows, not cards: the choice is one of eight, each needs one sentence, and a
/// row keeps the sentence readable at any width.
export function ChannelTypePicker({ onChange }: { onChange: (value: ChannelType) => void }) {
  const ui = useUiTranslations();
  const kinds = useChoosableChannelTypes();
  return (
    <div className="grid items-start gap-6 lg:grid-cols-2">
      {AUDIENCES.map(({ audience, title, hint }) => {
        const group = kinds.filter((info) => info.audience === audience);
        if (group.length === 0) return null;
        const headingId = `channel-group-${audience}`;
        return (
          <section key={audience} aria-labelledby={headingId}>
            <div className="mb-2.5 flex flex-wrap items-baseline gap-x-3 gap-y-1">
              <h2 id={headingId} className="text-[15px] font-semibold">
                {ui(title)}
              </h2>
              <p className="text-sm text-muted-foreground">{ui(hint)}</p>
            </div>
            <ul className="border bg-card">
              {group.map(({ kind, summary, needs }) => (
                <li key={kind} className="border-border/60 [&+&]:border-t">
                  <button
                    type="button"
                    onClick={() => onChange(kind)}
                    className="grid w-full grid-cols-[2rem_minmax(0,1fr)_auto] items-start gap-3.5 px-4 py-3.5 text-left transition-colors hover:bg-muted/50 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
                  >
                    <span className="flex size-8 items-center justify-center border bg-background">
                      <ChannelIcon kind={kind} className="icon-sharp size-4" />
                    </span>
                    <span className="min-w-0">
                      <span className="block font-semibold">{getChannelTypeDisplayName(kind)}</span>
                      <span className="mt-0.5 block max-w-[60ch] text-sm text-foreground/75">
                        {ui(summary)}
                      </span>
                    </span>
                    <span className="flex items-center gap-1.5 self-center text-xs text-muted-foreground">
                      {needs && <span className="hidden sm:inline">{ui(needs)}</span>}
                      <ChevronRight aria-hidden="true" className="size-3.5" />
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        );
      })}
    </div>
  );
}

/// The picked type, collapsed to one row above its form.
export function ChannelTypeChosen({
  kind,
  onChange,
}: {
  kind: ChannelType;
  onChange?: () => void;
}) {
  const ui = useUiTranslations();
  const info = channelTypeInfo(kind);
  return (
    <div className="flex items-center gap-3.5 border-b bg-muted/30 px-5 py-3.5">
      <span className="flex size-8 shrink-0 items-center justify-center border bg-background">
        <ChannelIcon kind={kind} className="icon-sharp size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-semibold">{getChannelTypeDisplayName(kind)}</p>
        {info && <p className="mt-0.5 text-sm text-foreground/75">{ui(info.summary)}</p>}
      </div>
      {onChange && (
        <Button type="button" variant="outline" size="sm" onClick={onChange}>
          {ui("Change type")}
        </Button>
      )}
    </div>
  );
}

/// What creating a channel leads to, so nobody is surprised that a new channel
/// starts closed. The steps are a real sequence, which is why they are numbered.
export function ChannelNextSteps({ kind }: { kind: ChannelType }) {
  const ui = useUiTranslations();
  const share: Partial<Record<ChannelType, [string, string]>> = {
    public_chat: [
      "Share the link",
      "Copy it from the channel page and send it to the people who should chat.",
    ],
    work_chat: ["Share the link", "Copy the chat site's address from the channel page."],
    slack: ["Add it to Slack", "Approve the agent's Slack app, then mention it in a channel."],
    voice: ["Call it", "Use the call button on the channel page or the microphone in chat."],
    api: ["Make an agent key", "Create a key on the channel page and call the URL shown there."],
  };
  const [lastTitle, lastText] = share[kind] ?? [
    "Connect your system",
    "The channel page shows the address to call and a sample request.",
  ];
  const steps: [string, string][] = [
    ["Create the channel", "It is saved as a draft. Nobody can reach the agent through it yet."],
    ["Publish it", "Publishing on the channel page opens it to callers."],
    [lastTitle, lastText],
  ];
  return (
    <section aria-labelledby="channel-next-steps">
      <h2 id="channel-next-steps" className="mb-3 text-xs font-medium text-muted-foreground">
        {ui("What happens next")}
      </h2>
      <ol className="space-y-3">
        {steps.map(([title, text], index) => (
          <li key={title} className="grid grid-cols-[1.375rem_minmax(0,1fr)] gap-2.5 text-sm">
            <span className="flex size-[1.375rem] items-center justify-center border bg-card font-mono text-xs font-medium">
              {index + 1}
            </span>
            <span>
              <span className="block font-semibold">{ui(title)}</span>
              <span className="block text-muted-foreground">{ui(text)}</span>
            </span>
          </li>
        ))}
      </ol>
    </section>
  );
}
