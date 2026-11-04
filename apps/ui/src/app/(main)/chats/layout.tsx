"use client";

import { Suspense, type ReactNode } from "react";
import { usePathname } from "next/navigation";
import { useOrg } from "@/providers/org-provider";
import { ChatWorkspace } from "@/components/chat/chat-workspace";
import { Skeleton } from "@/components/ui/skeleton";

export default function ChatsLayout({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const { currentOrg } = useOrg();
  if (pathname === "/chats/history") return children;
  return (
    <Suspense fallback={<Skeleton className="m-6 h-64" />}>
      <ChatWorkspace key={currentOrg?.public_id} />
    </Suspense>
  );
}
