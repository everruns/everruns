"use client";

import { use } from "react";
import { AppDetail } from "@/components/apps/app-detail";

export default function AppDetailPage({ params }: { params: Promise<{ appId: string }> }) {
  const { appId } = use(params);
  return <AppDetail appId={appId} />;
}
