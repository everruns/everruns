import { redirect } from "next/navigation";

// Plugins moved to Settings > Plugins; old links land there.
export default function PluginsRedirect() {
  redirect("/settings/plugins");
}
