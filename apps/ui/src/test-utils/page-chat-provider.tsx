import { useMemo, type ReactNode } from "react";
import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import { PageViewRegistryContext } from "@/components/chat-mode/page-view-state";

export function PageChatProvider({
  value,
  children,
}: {
  value: ChatModeState | null;
  children: ReactNode;
}) {
  const route = value?.page.route ?? "";
  const registry = useMemo(() => ({ route, register: () => () => {} }), [route]);
  return (
    <ChatModeContext.Provider value={value}>
      <PageViewRegistryContext.Provider value={registry}>
        {children}
      </PageViewRegistryContext.Provider>
    </ChatModeContext.Provider>
  );
}
