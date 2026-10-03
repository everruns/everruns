import {
  Braces,
  CalendarClock,
  Hash,
  MessageSquare,
  Monitor,
  Network,
  Webhook,
} from "lucide-react";
import { SlackIcon } from "@/components/icons/slack-icon";
import type { EndpointTransport } from "@/lib/api/types";

const icons = {
  slack: SlackIcon,
  schedule: CalendarClock,
  webhook: Webhook,
  ag_ui: Monitor,
  fcp: Hash,
  a2a: Network,
  api_endpoint: Braces,
  public_chat: MessageSquare,
};

export function ChannelIcon({ kind, className }: { kind: EndpointTransport; className?: string }) {
  const Icon = icons[kind];
  return <Icon className={className} aria-hidden="true" />;
}
