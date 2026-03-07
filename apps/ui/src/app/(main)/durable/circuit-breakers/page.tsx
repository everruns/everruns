"use client";

import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useCircuitBreakers } from "@/hooks";
import { forceOpenCircuitBreaker, forceCloseCircuitBreaker, deleteCircuitBreaker } from "@/lib/api";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Zap, ZapOff, Trash2, ShieldAlert, ShieldCheck, Clock } from "lucide-react";
import type { CircuitBreaker, CircuitBreakerState } from "@/lib/api/types";

function getStateBadgeVariant(state: CircuitBreakerState) {
  switch (state) {
    case "closed":
      return "default" as const;
    case "open":
      return "destructive" as const;
    case "half_open":
      return "secondary" as const;
    default:
      return "outline" as const;
  }
}

function getStateIcon(state: CircuitBreakerState) {
  switch (state) {
    case "closed":
      return <ShieldCheck className="h-4 w-4 text-green-500" />;
    case "open":
      return <ShieldAlert className="h-4 w-4 text-red-500" />;
    case "half_open":
      return <Clock className="h-4 w-4 text-yellow-500" />;
    default:
      return <Zap className="h-4 w-4" />;
  }
}

function formatDate(dateStr?: string): string {
  if (!dateStr) return "Never";
  const date = new Date(dateStr);
  return date.toLocaleString();
}

function CircuitBreakerCard({ breaker }: { breaker: CircuitBreaker }) {
  const queryClient = useQueryClient();
  const [openDialogOpen, setOpenDialogOpen] = useState(false);
  const [resetDialogOpen, setResetDialogOpen] = useState(false);

  const forceOpenMutation = useMutation({
    mutationFn: () => forceOpenCircuitBreaker(breaker.key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable-health"] });
      setOpenDialogOpen(false);
    },
  });

  const forceCloseMutation = useMutation({
    mutationFn: () => forceCloseCircuitBreaker(breaker.key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable-health"] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteCircuitBreaker(breaker.key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable-health"] });
      setResetDialogOpen(false);
    },
  });

  return (
    <Card className={breaker.state === "open" ? "border-red-500/50" : ""}>
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-2">
        <div className="flex items-center gap-2">
          {getStateIcon(breaker.state)}
          <div>
            <CardTitle className="text-base font-mono">{breaker.key}</CardTitle>
            <CardDescription className="text-xs">
              Updated {formatDate(breaker.updated_at)}
            </CardDescription>
          </div>
        </div>
        <Badge variant={getStateBadgeVariant(breaker.state)}>
          {breaker.state.replace("_", " ")}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* Stats */}
        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <span className="text-muted-foreground">Failure Count</span>
            <p className="font-medium">{breaker.failure_count}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Success Count</span>
            <p className="font-medium">{breaker.success_count}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Last Failure</span>
            <p className="font-medium text-xs">{formatDate(breaker.last_failure_at)}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Opened At</span>
            <p className="font-medium text-xs">{formatDate(breaker.opened_at)}</p>
          </div>
        </div>

        {/* Actions */}
        <div className="flex gap-2 pt-2 border-t">
          {breaker.state !== "open" ? (
            <>
              <Button
                variant="outline"
                size="sm"
                className="text-red-600 border-red-200 hover:bg-red-50"
                disabled={forceOpenMutation.isPending}
                onClick={() => setOpenDialogOpen(true)}
              >
                <ZapOff className="h-4 w-4 mr-1" />
                Force Open
              </Button>
              <Dialog open={openDialogOpen} onOpenChange={setOpenDialogOpen}>
                <DialogContent>
                  <DialogHeader>
                    <DialogTitle>Force Open Circuit Breaker?</DialogTitle>
                    <DialogDescription>
                      This will immediately block all calls through the &quot;{breaker.key}&quot;
                      circuit. Use this to proactively protect against a known issue.
                    </DialogDescription>
                  </DialogHeader>
                  <DialogFooter>
                    <Button variant="outline" onClick={() => setOpenDialogOpen(false)}>
                      Cancel
                    </Button>
                    <Button
                      variant="destructive"
                      onClick={() => forceOpenMutation.mutate()}
                      disabled={forceOpenMutation.isPending}
                    >
                      {forceOpenMutation.isPending ? "Opening..." : "Force Open"}
                    </Button>
                  </DialogFooter>
                </DialogContent>
              </Dialog>
            </>
          ) : (
            <Button
              variant="outline"
              size="sm"
              className="text-green-600 border-green-200 hover:bg-green-50"
              onClick={() => forceCloseMutation.mutate()}
              disabled={forceCloseMutation.isPending}
            >
              <ShieldCheck className="h-4 w-4 mr-1" />
              {forceCloseMutation.isPending ? "Closing..." : "Force Close"}
            </Button>
          )}

          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground hover:text-destructive"
            disabled={deleteMutation.isPending}
            onClick={() => setResetDialogOpen(true)}
          >
            <Trash2 className="h-4 w-4 mr-1" />
            Reset
          </Button>
          <Dialog open={resetDialogOpen} onOpenChange={setResetDialogOpen}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Reset Circuit Breaker?</DialogTitle>
                <DialogDescription>
                  This will delete the circuit breaker state for &quot;{breaker.key}&quot;. A new
                  circuit breaker will be created automatically when needed.
                </DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button variant="outline" onClick={() => setResetDialogOpen(false)}>
                  Cancel
                </Button>
                <Button onClick={() => deleteMutation.mutate()} disabled={deleteMutation.isPending}>
                  {deleteMutation.isPending ? "Resetting..." : "Reset"}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      </CardContent>
    </Card>
  );
}

export default function CircuitBreakersPage() {
  const { data, isLoading, error } = useCircuitBreakers();

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Circuit Breakers</h1>
        </div>
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {[...Array(3)].map((_, i) => (
              <Skeleton key={i} className="h-48" />
            ))}
          </div>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Circuit Breakers</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <ShieldAlert className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Failed to Load Circuit Breakers</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md">
              {error instanceof Error ? error.message : "An error occurred"}
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const circuitBreakers = data?.data || [];
  const openBreakers = circuitBreakers.filter((cb) => cb.state === "open");
  const closedBreakers = circuitBreakers.filter((cb) => cb.state === "closed");
  const halfOpenBreakers = circuitBreakers.filter((cb) => cb.state === "half_open");

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Circuit Breakers</h1>
      </div>
      <div className="space-y-6">
        {/* Summary Cards */}
        <div className="grid gap-4 md:grid-cols-3">
          <Card>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">Total</CardTitle>
              <Zap className="h-4 w-4 text-muted-foreground" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold">{circuitBreakers.length}</div>
              <p className="text-xs text-muted-foreground">circuit breakers</p>
            </CardContent>
          </Card>
          <Card className={openBreakers.length > 0 ? "border-red-500/50" : ""}>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">Open</CardTitle>
              <ShieldAlert className="h-4 w-4 text-red-500" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-red-600">{openBreakers.length}</div>
              <p className="text-xs text-muted-foreground">blocking calls</p>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">Closed</CardTitle>
              <ShieldCheck className="h-4 w-4 text-green-500" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-green-600">{closedBreakers.length}</div>
              <p className="text-xs text-muted-foreground">allowing calls</p>
            </CardContent>
          </Card>
        </div>

        {/* Circuit Breaker List */}
        {circuitBreakers.length > 0 ? (
          <div className="space-y-4">
            {/* Open breakers first */}
            {openBreakers.length > 0 && (
              <div>
                <h3 className="text-lg font-semibold mb-3 flex items-center gap-2">
                  <ShieldAlert className="h-5 w-5 text-red-500" />
                  Open Circuit Breakers
                </h3>
                <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                  {openBreakers.map((cb) => (
                    <CircuitBreakerCard key={cb.key} breaker={cb} />
                  ))}
                </div>
              </div>
            )}

            {/* Half-open breakers */}
            {halfOpenBreakers.length > 0 && (
              <div>
                <h3 className="text-lg font-semibold mb-3 flex items-center gap-2">
                  <Clock className="h-5 w-5 text-yellow-500" />
                  Half-Open Circuit Breakers
                </h3>
                <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                  {halfOpenBreakers.map((cb) => (
                    <CircuitBreakerCard key={cb.key} breaker={cb} />
                  ))}
                </div>
              </div>
            )}

            {/* Closed breakers */}
            {closedBreakers.length > 0 && (
              <div>
                <h3 className="text-lg font-semibold mb-3 flex items-center gap-2">
                  <ShieldCheck className="h-5 w-5 text-green-500" />
                  Closed Circuit Breakers
                </h3>
                <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                  {closedBreakers.map((cb) => (
                    <CircuitBreakerCard key={cb.key} breaker={cb} />
                  ))}
                </div>
              </div>
            )}
          </div>
        ) : (
          <Card>
            <CardContent className="flex flex-col items-center justify-center py-12">
              <Zap className="h-12 w-12 text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No Circuit Breakers</h3>
              <p className="text-sm text-muted-foreground text-center max-w-md">
                Circuit breakers are created automatically when protected operations are executed.
                Check back after running some workflows.
              </p>
            </CardContent>
          </Card>
        )}

        {/* Info Card */}
        <Card>
          <CardHeader>
            <CardTitle className="text-base">About Circuit Breakers</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground space-y-2">
            <p>
              Circuit breakers protect against cascading failures when external services (like LLM
              providers) are unavailable.
            </p>
            <ul className="list-disc list-inside space-y-1">
              <li>
                <strong>Closed:</strong> Normal operation - calls are allowed through
              </li>
              <li>
                <strong>Open:</strong> Service failing - calls are rejected immediately
              </li>
              <li>
                <strong>Half-Open:</strong> Testing recovery - allowing one call to check if service
                is back
              </li>
            </ul>
            <p className="pt-2">
              Use <strong>Force Open</strong> to proactively block calls when you know a service has
              issues. Use <strong>Force Close</strong> to immediately allow calls after a service
              recovers.
            </p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
