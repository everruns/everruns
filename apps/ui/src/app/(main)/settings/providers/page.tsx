import { redirect } from "next/navigation";

// Providers moved from Settings to Registries → Models (Providers tab); keep old
// links and bookmarks working.
export default function ProvidersSettingsPage() {
  redirect("/models?tab=providers");
}
