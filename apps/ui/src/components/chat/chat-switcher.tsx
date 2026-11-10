"use client";

// "Main chat ▾" beside a page: the sidebar is hidden there, so this is how the
// person reaches side chats and the current chat's threads without leaving the
// page (knowledge/ui/chat-mode.md). Selecting a chat keeps the page as it is;
// a thread opens in the page pane.

import Link from "next/link";
import { ChevronDown, Plus } from "lucide-react";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { buttonVariants } from "@/components/ui/button";
import { assignmentGroup, coordinatorThreads, threadRowTitle } from "@/lib/chat-thread-work";
import { threadTitle } from "@/lib/chat-threads";
import { cn } from "@/lib/utils";
import { useUiTranslations } from "@/providers/locale-provider";

/** Side chats listed in the switcher; the rest are under View all chats. */
const SWITCHER_SIDE_CHATS = 6;

export function ChatSwitcher({
  mainChatId,
  activeChatId,
  onSelectChat,
  onSelectThread,
}: {
  mainChatId: string;
  activeChatId: string;
  onSelectChat: (id: string) => void;
  onSelectThread: (id: string) => void;
}) {
  const ui = useUiTranslations();
  const { threads: chats } = useChatThreads({ limit: SWITCHER_SIDE_CHATS + 1 });
  const sideChats = chats.filter((chat) => chat.id !== mainChatId).slice(0, SWITCHER_SIDE_CHATS);
  const tasks = useSessionTasks(activeChatId);
  const threads = coordinatorThreads(tasks.data ?? []).filter(
    (thread) => assignmentGroup(thread.latest) !== "Resolved",
  );
  const active = chats.find((chat) => chat.id === activeChatId);
  const label = activeChatId === mainChatId || !active ? ui("Main chat") : threadTitle(active);

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger
        aria-label={ui("Switch chat")}
        className={cn(buttonVariants({ variant: "ghost", size: "sm" }), "max-w-48")}
      >
        <span className="truncate">{label}</span>
        <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner side="bottom" align="end" sideOffset={4}>
        <DropdownMenuContent aria-label={ui("Switch chat")} className="w-80">
          <DropdownMenuItem
            aria-current={activeChatId === mainChatId ? "true" : undefined}
            onClick={() => onSelectChat(mainChatId)}
          >
            <span className="font-medium">{ui("Main chat")}</span>
          </DropdownMenuItem>
          {threads.length > 0 && (
            <DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>{ui("Threads of this chat")}</DropdownMenuLabel>
              {threads.map((thread) => (
                <DropdownMenuItem
                  key={thread.threadId}
                  onClick={() => onSelectThread(thread.threadId)}
                >
                  <span className="truncate">{threadRowTitle(thread)}</span>
                  <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                    {ui(assignmentGroup(thread.latest))}
                  </span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
          )}
          <DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>{ui("Side chats")}</DropdownMenuLabel>
            {sideChats.map((chat) => (
              <DropdownMenuItem
                key={chat.id}
                aria-current={chat.id === activeChatId ? "true" : undefined}
                onClick={() => onSelectChat(chat.id)}
              >
                <span className="truncate">{threadTitle(chat)}</span>
              </DropdownMenuItem>
            ))}
            <DropdownMenuItem render={<Link href="/chats/new" prefetch={false} />}>
              <Plus aria-hidden="true" className="size-3.5" />
              {ui("New side chat")}
            </DropdownMenuItem>
            <DropdownMenuItem
              render={<Link href="/chats/history" prefetch={false} />}
              className="text-muted-foreground"
            >
              {ui("View all chats")}
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
