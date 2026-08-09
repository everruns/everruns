import type { Metadata } from "next";
import { formatPageTitle } from "@/lib/page-title";
import NewChatPageClient from "./new-chat-page-client";

export const metadata: Metadata = {
  title: formatPageTitle("New chat", "Chats"),
};

export default function NewChatPage() {
  return <NewChatPageClient />;
}
