/**
 * Icon for each message origin, shared by Chat and the session trace so a
 * scheduled run looks the same in both.
 */
import {
  CalendarClock,
  Cog,
  RefreshCw,
  SmilePlus,
  Webhook,
  Zap,
  type LucideIcon,
} from "lucide-react";
import type { MessageOriginKind } from "@/lib/message-origin";

export const ORIGIN_ICON: Record<MessageOriginKind, LucideIcon> = {
  schedule: CalendarClock,
  channel_schedule: CalendarClock,
  agent_trigger: Zap,
  webhook: Webhook,
  task_wake: RefreshCw,
  reaction: SmilePlus,
  system: Cog,
};
