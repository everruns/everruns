/**
 * The honest disabled state for the Chats surface. Chats is the landing route,
 * so a disabled `global_chat` flag has to explain itself and offer somewhere to
 * go — never a blank page or a spinner that never resolves (EVE-713).
 */
"use client";

import Link from "next/link";
import { MessageCircle } from "lucide-react";
import { EmptyState } from "@/components/layout";
import { buttonVariants } from "@/components/ui/button";

export function ChatsDisabled() {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <EmptyState
        className="w-full max-w-lg"
        icon={<MessageCircle />}
        title="Chats is not enabled"
        description="The global_chat feature is turned off for this organization. Sessions still record everything your agents run."
        action={
          <Link href="/sessions" className={buttonVariants({ variant: "outline" })}>
            Go to Sessions
          </Link>
        }
      />
    </div>
  );
}
