"use client";

import { useHarnesses, useCapabilities } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Plus } from "lucide-react";
import { HarnessCard } from "@/components/harnesses";

export default function HarnessesPage() {
  const { data: harnesses, isLoading, error } = useHarnesses();
  const { data: allCapabilities } = useCapabilities();

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Error loading harnesses: {error.message}</div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Harnesses</h1>
        <Link href="/harnesses/new">
          <Button variant="accent">
            <Plus className="w-4 h-4 mr-2" />
            New Harness
          </Button>
        </Link>
      </div>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[...Array(6)].map((_, i) => (
            <Card key={i}>
              <CardHeader>
                <Skeleton className="h-6 w-3/4" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-4 w-full mb-2" />
                <Skeleton className="h-4 w-2/3" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : harnesses?.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-muted-foreground mb-4">No harnesses yet</p>
          <Link href="/harnesses/new">
            <Button>
              <Plus className="w-4 h-4 mr-2" />
              Create your first harness
            </Button>
          </Link>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {harnesses?.map((harness) => (
            <HarnessCard
              key={harness.id}
              harness={harness}
              allCapabilities={allCapabilities}
              showEditButton
            />
          ))}
        </div>
      )}
    </div>
  );
}
