import { redirect } from "next/navigation";

// Editing happens in place on the harness page now; keep old links working.
export default async function EditHarnessPage({
  params,
}: {
  params: Promise<{ harnessId: string }>;
}) {
  const { harnessId } = await params;
  redirect(`/harnesses/${harnessId}?mode=edit`);
}
