"use client";

import { use } from "react";
import { AppDetailV2 } from "@/components/apps/app-detail-v2";

export default function AppDetailPage({ params }: { params: Promise<{ appId: string }> }) {
  const { appId } = use(params);
  return <AppDetailV2 appId={appId} />;
}
