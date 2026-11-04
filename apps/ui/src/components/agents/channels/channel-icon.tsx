import {
  AppWindow,
  CalendarClock,
  Code,
  Globe,
  Handshake,
  MessageSquareText,
  MessagesSquare,
  Mic,
  Network,
  Webhook,
} from "lucide-react";
import { SlackIcon } from "@/components/icons/slack-icon";
import type { ChannelType } from "@/lib/api/types";

const icons = {
  slack: SlackIcon,
  schedule: CalendarClock,
  webhook: Webhook,
  ag_ui: AppWindow,
  fcp: MessageSquareText,
  a2a: Network,
  api: Code,
  work_chat: MessagesSquare,
  public_chat: Globe,
  voice: Mic,
  poppy: Handshake,
};

export function ChannelIcon({ kind, className }: { kind: ChannelType; className?: string }) {
  const Icon = icons[kind];
  return <Icon className={className} aria-hidden="true" />;
}
