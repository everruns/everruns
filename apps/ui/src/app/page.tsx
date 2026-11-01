import { redirect } from "next/navigation";

// Chat is the landing route: the first thing in the nav and the default
// destination for a user with no other intent
// (knowledge/ui/information-architecture.md). `?landing` lets the Chat
// workspace open that Chat beside Home instead (knowledge/ui/org-home.md).
export default function Home() {
  // Matches LANDING_PARAM in components/chat-mode/landing.ts (a client module).
  redirect("/chats?landing");
}
