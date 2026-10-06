import { redirect } from "next/navigation";

// Approvals are read on the session they belong to. The org-wide list showed
// a request and its grant as two rows, which did not say what was approved.
export default function ApprovalsPage() {
  redirect("/sessions");
}
