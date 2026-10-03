"use client";

import { PlaygroundSetup } from "@/components/playground/playground-setup";
import { useOrg } from "@/providers/org-provider";

export default function NewPlaygroundPage() {
  const { currentOrg } = useOrg();
  return <PlaygroundSetup key={currentOrg?.public_id} />;
}
