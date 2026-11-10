import type { ReactNode } from "react";

// The (main) ChatModeShell renders the conversation for every /chats route
// (full screen); the route pages only exist so the URLs resolve.
export default function ChatsLayout({ children }: { children: ReactNode }) {
  return children;
}
