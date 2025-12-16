"use client";

import { useHarnesses, useDeleteHarness } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Plus, Trash2 } from "lucide-react";

export default function HarnessesPage() {
  const { data: harnesses, isLoading, error } = useHarnesses();
  const deleteHarness = useDeleteHarness();

  const handleDelete = async (harnessId: string) => {
    if (confirm("Are you sure you want to archive this harness?")) {
      await deleteHarness.mutateAsync(harnessId);
    }
  };

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">
          Error loading harnesses: {error.message}
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Harnesses</h1>
        <Link href="/harnesses/new">
          <Button>
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
            <Card key={harness.id} className="hover:shadow-md transition-shadow">
              <CardHeader className="flex flex-row items-start justify-between space-y-0">
                <div>
                  <CardTitle className="text-lg">
                    <Link
                      href={`/harnesses/${harness.id}`}
                      className="hover:underline"
                    >
                      {harness.display_name}
                    </Link>
                  </CardTitle>
                  <p className="text-sm text-muted-foreground font-mono">
                    {harness.slug}
                  </p>
                </div>
                <Badge variant={harness.status === "active" ? "default" : "secondary"}>
                  {harness.status}
                </Badge>
              </CardHeader>
              <CardContent>
                <p className="text-sm text-muted-foreground mb-4 line-clamp-2">
                  {harness.description || "No description"}
                </p>
                {harness.tags.length > 0 && (
                  <div className="flex flex-wrap gap-1 mb-4">
                    {harness.tags.map((tag) => (
                      <Badge key={tag} variant="outline" className="text-xs">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                )}
                <div className="flex items-center justify-between">
                  <span className="text-xs text-muted-foreground">
                    Created {new Date(harness.created_at).toLocaleDateString()}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="text-destructive hover:text-destructive"
                    onClick={() => handleDelete(harness.id)}
                    disabled={deleteHarness.isPending}
                  >
                    <Trash2 className="w-4 h-4" />
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
