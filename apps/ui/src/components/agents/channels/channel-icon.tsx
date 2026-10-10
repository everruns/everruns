import {
  Braces,
  CalendarClock,
  Handshake,
  Hash,
  KeyRound,
  MessageSquare,
  Mic,
  Monitor,
  Network,
  Webhook,
} from "lucide-react";
import { SlackIcon } from "@/components/icons/slack-icon";
import type { ChannelType } from "@/lib/api/types";

const icons = {
  slack: SlackIcon,
  schedule: CalendarClock,
  webhook: Webhook,
  ag_ui: Monitor,
  fcp: Hash,
  a2a: Network,
  api_endpoint: Braces,
  api: KeyRound,
  public_chat: MessageSquare,
  voice: Mic,
  poppy: Handshake,
};

export function ChannelIcon({ kind, className }: { kind: ChannelType; className?: string }) {
  const Icon = icons[kind];
  return <Icon className={className} aria-hidden="true" />;
}
