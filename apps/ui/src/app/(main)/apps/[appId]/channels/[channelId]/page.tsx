"use client";

import { use } from "react";
import { useApp } from "@/hooks/use-apps";
import { ChannelEditor } from "@/components/apps/channel-editor";

export default function EditChannelPage({
  params,
}: {
  params: Promise<{ appId: string; channelId: string }>;
}) {
  const { appId, channelId } = use(params);
  const { data: app } = useApp(appId);
  const appName = app?.name ?? "app";

  return (
    <ChannelEditor
      appId={appId}
      channelId={channelId}
      nav={{
        breadcrumbs: [
          { label: "Apps", href: "/apps" },
          { label: appName, href: `/apps/${appId}` },
        ],
        returnHref: `/apps/${appId}`,
        returnLabel: appName,
      }}
    />
  );
}
