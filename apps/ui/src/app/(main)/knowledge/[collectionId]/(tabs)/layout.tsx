import { Suspense, type ReactNode } from "react";
import { CollectionShell } from "./collection-shell";

export default async function CollectionTabsLayout({
  children,
  params,
}: {
  children: ReactNode;
  params: Promise<{ collectionId: string }>;
}) {
  const { collectionId } = await params;
  return (
    <Suspense>
      <CollectionShell collectionId={collectionId}>{children}</CollectionShell>
    </Suspense>
  );
}
