import type { ChannelType } from "@/lib/api/types";

export function getChannelTypeDisplayName(channelType: ChannelType): string {
  switch (channelType) {
    case "ag_ui":
      return "AG-UI";
    case "slack":
      return "Slack";
  }
}
