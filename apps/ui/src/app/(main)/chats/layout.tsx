"use client";

import { Suspense, type ReactNode } from "react";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useOrg } from "@/providers/org-provider";
import { ChatWorkspace } from "@/components/chat/chat-workspace";
import { Skeleton } from "@/components/ui/skeleton";

export default function ChatsLayout({ children }: { children: ReactNode }) {
  const enabled = useFeatureFlag("chat_threads");
  const { currentOrg } = useOrg();
  if (!enabled) return children;
  return (
    <Suspense fallback={<Skeleton className="m-6 h-64" />}>
      <ChatWorkspace key={currentOrg?.public_id} />
    </Suspense>
  );
}
