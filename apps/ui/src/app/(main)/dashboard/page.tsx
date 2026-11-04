import { redirect } from "next/navigation";

// The old Dashboard's replacement is Home (knowledge/ui/org-home.md). Kept as a
// redirect so existing links and bookmarks land on it.
export default function DashboardPage() {
  redirect("/home");
}
