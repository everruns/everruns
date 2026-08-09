import { redirect } from "next/navigation";

// The single-session Platform Chat shell became the Chats surface (EVE-851).
// Kept as a redirect so existing links and bookmarks land somewhere useful.
export default function ChatPage() {
  redirect("/chats");
}
