import type { Metadata } from "next";
import { formatPageTitle } from "@/lib/page-title";
import ChatsPageClient from "./chats-page-client";

export const metadata: Metadata = {
  title: formatPageTitle("Chats"),
};

export default function ChatsPage() {
  return <ChatsPageClient />;
}
