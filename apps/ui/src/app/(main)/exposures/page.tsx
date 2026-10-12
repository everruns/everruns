import { redirect } from "next/navigation";

// Channels live on the Agents page now; keep old Exposures links working.
export default function ExposuresPage() {
  redirect("/agents?view=channels");
}
