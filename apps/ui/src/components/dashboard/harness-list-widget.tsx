"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Boxes, Plus } from "lucide-react";
import type { Harness } from "@/lib/api/types";

interface HarnessListWidgetProps {
  harnesses: Harness[];
}

export function HarnessListWidget({ harnesses }: HarnessListWidgetProps) {
  const activeHarnesses = harnesses.filter((h) => h.status === "active").slice(0, 5);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Active Harnesses</CardTitle>
        <Link href="/harnesses/new">
          <Button variant="outline" size="sm">
            <Plus className="h-4 w-4 mr-1" />
            New Harness
          </Button>
        </Link>
      </CardHeader>
      <CardContent>
        {activeHarnesses.length === 0 ? (
          <div className="text-center py-8">
            <Boxes className="h-12 w-12 mx-auto text-muted-foreground mb-2" />
            <p className="text-muted-foreground">No harnesses yet.</p>
            <Link href="/harnesses/new">
              <Button variant="link">Create your first harness</Button>
            </Link>
          </div>
        ) : (
          <div className="space-y-3">
            {activeHarnesses.map((harness) => (
              <Link
                key={harness.id}
                href={`/harnesses/${harness.id}`}
                className="flex items-center justify-between p-3 rounded-lg border hover:bg-accent transition-colors"
              >
                <div className="flex items-center gap-3">
                  <Boxes className="h-5 w-5 text-muted-foreground" />
                  <div>
                    <p className="font-medium">{harness.display_name}</p>
                    <p className="text-xs text-muted-foreground font-mono">
                      {harness.slug}
                    </p>
                  </div>
                </div>
                <Badge
                  variant="outline"
                  className="bg-green-100 text-green-800"
                >
                  Active
                </Badge>
              </Link>
            ))}
            {harnesses.length > 5 && (
              <Link href="/harnesses">
                <Button variant="ghost" className="w-full">
                  View all {harnesses.length} harnesses
                </Button>
              </Link>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
