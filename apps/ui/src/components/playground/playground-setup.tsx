"use client";

import { useState } from "react";
import { useSearchParams } from "next/navigation";
import { Users, FlaskConical } from "lucide-react";
import { NewChatForm } from "@/components/chat/new-chat-form";
import { VirtualUserSelect } from "@/components/virtual-user/virtual-user-select";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { useOrg } from "@/providers/org-provider";
import { usePageTitle } from "@/hooks";
import {
  BackLink,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageFooter,
  PageMain,
  PageMasthead,
  PageRail,
  RailSection,
} from "@/components/layout/page-layout";

export function PlaygroundSetup() {
  const { currentOrg } = useOrg();
  return <Setup key={currentOrg?.public_id} />;
}

function Setup() {
  const searchParams = useSearchParams();
  const { currentOrg, hasRole } = useOrg();
  const { data: me, error } = useVirtualUser("me");
  const [selected, setSelected] = useState("");
  const subject = selected || me?.id || "";
  usePageTitle("New Playground chat", "Playground");
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Playground", href: "/playground" }, { label: "New Playground chat" }]}
      />
      <PageMasthead
        icon={<FlaskConical />}
        title="New Playground chat"
        description="Choose an agent and a virtual user to start a shared chat."
      />
      <PageColumns>
        <PageMain>
          <div className="border bg-card p-6">
            <div className="max-w-lg">
              <NewChatForm
                surface="playground"
                endUserId={subject}
                initialAgentId={searchParams.get("agent") ?? undefined}
              >
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
                    The agent and virtual user stay fixed for this chat.
                  </p>
                  {error && (
                    <p role="alert" className="text-xs text-destructive">
                      Could not load your virtual user. Please reload.
                    </p>
                  )}
                </div>
              </NewChatForm>
            </div>
          </div>
        </PageMain>
        <PageRail>
          <RailSection label="Sharing">
            <p className="flex items-center gap-2 text-sm font-medium">
              <Users className="size-4" /> Shared with your organisation
            </p>
            <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
              Everyone in {currentOrg?.name ?? "your organisation"} can view the chat. Private user
              connections and memory are unavailable in Playground.
            </p>
          </RailSection>
        </PageRail>
      </PageColumns>
      <PageFooter>
        <BackLink href="/playground">Back to Playground</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
