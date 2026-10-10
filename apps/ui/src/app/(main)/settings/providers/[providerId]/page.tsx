import { redirect } from "next/navigation";

// Provider pages moved under Registries → Models; keep old links working.
export default async function ProviderSettingsPage({
  params,
}: {
  params: Promise<{ providerId: string }>;
}) {
  const { providerId } = await params;
  redirect(`/models/providers/${encodeURIComponent(providerId)}`);
}
