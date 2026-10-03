"use client";

import { useState } from "react";
import { Users, FlaskConical } from "lucide-react";
import { NewChatForm } from "@/components/chat/new-chat-form";
import { VirtualUserSelect } from "@/components/virtual-user/virtual-user-select";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { useOrg } from "@/providers/org-provider";
import { usePageTitle } from "@/hooks";

export function PlaygroundSetup() {
  const { currentOrg, hasRole } = useOrg();
  const { data: me, error } = useVirtualUser("me");
  const [selected, setSelected] = useState("");
  const subject = selected || me?.id || "";
  usePageTitle("New conversation", "Playground");
  return (
    <div key={currentOrg?.public_id} className="flex h-full flex-col bg-background bg-brand-dots">
      <header className="border-b bg-background/80 px-6 py-4">
        <h1 className="text-lg font-semibold">New Playground conversation</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Test an agent together with your organisation.
        </p>
      </header>
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        <div className="flex min-h-64 flex-1 items-center justify-center p-8 text-center">
          <div className="max-w-md space-y-3">
            <FlaskConical className="mx-auto size-8 text-primary" />
            <h2 className="text-xl font-semibold tracking-tight">A place to try things</h2>
            <p className="text-sm leading-relaxed text-muted-foreground">
              Choose an agent and the virtual user this conversation is about. Start a conversation,
              then send your first message.
            </p>
          </div>
        </div>
        <aside className="w-full space-y-6 border-t bg-background/90 p-6 lg:w-80 lg:border-l lg:border-t-0">
          <div>
            <h2 className="mb-3 text-xs font-medium uppercase tracking-widest text-muted-foreground">
              Conversation setup
            </h2>
            <NewChatForm surface="playground" endUserId={subject}>
              <div className="space-y-2">
                <p className="text-xs font-medium text-muted-foreground">Talk as</p>
                <VirtualUserSelect
                  usage="end_user"
                  value={subject}
                  onValueChange={setSelected}
                  includeNone={false}
                  disabled={!hasRole("admin")}
                  placeholder="Talk as virtual user"
                  className="w-full"
                />
                <p className="text-xs leading-relaxed text-muted-foreground">
                  The agent and virtual user stay fixed for this conversation.
                </p>
                {error && (
                  <p role="alert" className="text-xs text-destructive">
                    Could not load your virtual user. Please reload.
                  </p>
                )}
              </div>
            </NewChatForm>
          </div>
          <div className="border-t pt-4 text-sm">
            <p className="flex items-center gap-2 font-medium">
              <Users className="size-4" /> Shared with your organisation
            </p>
            <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
              Everyone in {currentOrg?.name ?? "your organisation"} can view the conversation.
              Private user connections and memory are unavailable in Playground.
            </p>
          </div>
        </aside>
      </div>
    </div>
  );
}
