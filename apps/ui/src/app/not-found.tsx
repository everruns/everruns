import type { Metadata } from "next";
import { ResourceNotFound } from "@/components/resource-not-found";
import { formatPageTitle } from "@/lib/page-title";

export const metadata: Metadata = {
  title: formatPageTitle("Page not found"),
};

export default function NotFound() {
  return (
    <ResourceNotFound
      title="Page not found"
      description="This page does not exist in the current workspace."
      backHref="/chats"
      backLabel="Back to dashboard"
      className="min-h-screen bg-background bg-brand-dots"
    />
  );
}
