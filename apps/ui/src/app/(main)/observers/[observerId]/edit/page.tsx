"use client";

import { use } from "react";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { useObserver, usePageTitle } from "@/hooks";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { ObserverForm } from "@/components/observers/observer-form";

export default function EditObserverPage({ params }: { params: Promise<{ observerId: string }> }) {
  const { observerId } = use(params);
  const observersEnabled = useFeatureFlag("observers");
  const { data: observer, isLoading } = useObserver(observerId);
  usePageTitle(observer ? `Edit ${observer.name}` : "Edit Observer", "Observers");

  if (!observersEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">Observers is not enabled</p>
          <p className="text-sm">This feature is currently disabled.</p>
        </div>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!observer) {
    return (
      <ResourceNotFound
        title="Observer not found"
        description="This observer may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/observers"
        backLabel="Back to observers"
        resourceId={observerId}
      />
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href={`/observers/${observerId}`}
        className="mb-6 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to observer
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Edit Observer</CardTitle>
        </CardHeader>
        <CardContent>
          <ObserverForm observer={observer} />
        </CardContent>
      </Card>
    </div>
  );
}
