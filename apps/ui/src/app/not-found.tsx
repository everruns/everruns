import { ResourceNotFound } from "@/components/resource-not-found";

export default function NotFound() {
  return (
    <ResourceNotFound
      title="Page not found"
      description="This page does not exist in the current workspace."
      backHref="/dashboard"
      backLabel="Back to dashboard"
      className="min-h-screen bg-background bg-brand-dots"
    />
  );
}
