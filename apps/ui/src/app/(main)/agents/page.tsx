"use client";

import { Suspense } from "react";
import { AgentsHome } from "@/components/agents/home/agents-home";

export default function AgentsPage() {
  return (
    <Suspense fallback={null}>
      <AgentsHome />
    </Suspense>
  );
}
