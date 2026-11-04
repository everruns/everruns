"use client";
import { Suspense } from "react";
import { McpWorkspacePage } from "@/components/mcp-app/workspace-page";
export default function McpAppPage() {
  return (
    <Suspense>
      <McpWorkspacePage />
    </Suspense>
  );
}
