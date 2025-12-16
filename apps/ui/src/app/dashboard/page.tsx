"use client";

import { useHarnesses } from "@/hooks";
import { Header } from "@/components/layout/header";
import { StatsCards } from "@/components/dashboard/stats-cards";
import { HarnessListWidget } from "@/components/dashboard/harness-list-widget";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus, Boxes } from "lucide-react";

export default function DashboardPage() {
  const { data: harnesses = [], isLoading: harnessesLoading } = useHarnesses();

  if (harnessesLoading) {
    return (
      <>
        <Header title="Dashboard" />
        <div className="p-6 space-y-6">
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            {[...Array(4)].map((_, i) => (
              <Skeleton key={i} className="h-32" />
            ))}
          </div>
          <div className="grid gap-6 md:grid-cols-2">
            <Skeleton className="h-96" />
            <Skeleton className="h-96" />
          </div>
        </div>
      </>
    );
  }

  // For now, pass empty sessions array since we don't have a global sessions endpoint yet
  const sessions: [] = [];

  return (
    <>
      <Header title="Dashboard" />
      <div className="p-6 space-y-6">
        <StatsCards harnesses={harnesses} sessions={sessions} />
        <div className="grid gap-6 md:grid-cols-2">
          <HarnessListWidget harnesses={harnesses} />

          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Quick Actions</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <Link href="/harnesses/new" className="block">
                <Button variant="outline" className="w-full justify-start">
                  <Plus className="h-4 w-4 mr-2" />
                  Create New Harness
                </Button>
              </Link>
              <Link href="/harnesses" className="block">
                <Button variant="outline" className="w-full justify-start">
                  <Boxes className="h-4 w-4 mr-2" />
                  Browse All Harnesses
                </Button>
              </Link>
              {harnesses.length > 0 && (
                <p className="text-sm text-muted-foreground">
                  Select a harness to view its sessions and start conversations.
                </p>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </>
  );
}
