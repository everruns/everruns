import { redirect } from "next/navigation";

// The Dashboard existed because the old IA had nowhere else to put a landing
// page. Chats is the landing surface now (EVE-851), and the widgets this page
// carried are covered by Chats, Sessions and session detail (EVE-855). Kept as
// a redirect so existing links and bookmarks land somewhere useful.
export default function DashboardPage() {
  redirect("/chats");
}
