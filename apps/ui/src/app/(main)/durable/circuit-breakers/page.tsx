"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useDurablePageRows } from "@/components/chat-mode/use-durable-page-rows";

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
import { useCircuitBreakers, usePageTitle } from "@/hooks";
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
      return <ShieldCheck className="h-4 w-4 text-success" />;
    case "open":
      return <ShieldAlert className="h-4 w-4 text-destructive" />;
    case "half_open":
      return <Clock className="h-4 w-4 text-warning" />;
    default:
      return <Zap className="h-4 w-4" />;
  }
}

function formatDate(dateStr?: string, locale?: string): string {
  if (!dateStr) return "Never";
  const date = new Date(dateStr);
  return date.toLocaleString(locale);
}

function CircuitBreakerCard({ breaker }: { breaker: CircuitBreaker }) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
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
    <Card className={breaker.state === "open" ? "border-destructive/50" : ""}>
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-2">
        <div className="flex items-center gap-2">
          {getStateIcon(breaker.state)}
          <div>
            <CardTitle className="text-base font-mono">{breaker.key}</CardTitle>
            <CardDescription className="text-xs">
              {ui("Updated")} {formatDate(breaker.updated_at, displayLocale)}
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
            <span className="text-muted-foreground">{ui("Failure Count")}</span>
            <p className="font-medium">{breaker.failure_count}</p>
          </div>
          <div>
            <span className="text-muted-foreground">{ui("Success Count")}</span>
            <p className="font-medium">{breaker.success_count}</p>
          </div>
          <div>
            <span className="text-muted-foreground">{ui("Last Failure")}</span>
            <p className="font-medium text-xs">
              {formatDate(breaker.last_failure_at, displayLocale)}
            </p>
          </div>
          <div>
            <span className="text-muted-foreground">{ui("Opened At")}</span>
            <p className="font-medium text-xs">{formatDate(breaker.opened_at, displayLocale)}</p>
          </div>
        </div>

        {/* Actions */}
        <div className="flex gap-2 pt-2 border-t">
          {breaker.state !== "open" ? (
            <>
              <Button
                variant="outline"
                size="sm"
                className="text-destructive border-destructive/30 hover:bg-destructive/10"
                disabled={forceOpenMutation.isPending}
                onClick={() => setOpenDialogOpen(true)}
              >
                <ZapOff className="h-4 w-4 mr-1" />
                {ui("Force Open")}
              </Button>
              <Dialog open={openDialogOpen} onOpenChange={setOpenDialogOpen}>
                <DialogContent>
                  <DialogHeader>
                    <DialogTitle>{ui("Force Open Circuit Breaker?")}</DialogTitle>
                    <DialogDescription>
                      {ui('This will immediately block all calls through the "')}
                      {breaker.key}
                      {ui('" circuit. Use this to proactively protect against a known issue.')}
                    </DialogDescription>
                  </DialogHeader>
                  <DialogFooter>
                    <Button variant="outline" onClick={() => setOpenDialogOpen(false)}>
                      {ui("Cancel")}
                    </Button>
                    <Button
                      variant="destructive"
                      onClick={() => forceOpenMutation.mutate()}
                      disabled={forceOpenMutation.isPending}
                    >
                      {forceOpenMutation.isPending ? ui("Opening...") : ui("Force Open")}
                    </Button>
                  </DialogFooter>
                </DialogContent>
              </Dialog>
            </>
          ) : (
            <Button
              variant="outline"
              size="sm"
              className="text-success border-success/30 hover:bg-success/10"
              onClick={() => forceCloseMutation.mutate()}
              disabled={forceCloseMutation.isPending}
            >
              <ShieldCheck className="h-4 w-4 mr-1" />
              {forceCloseMutation.isPending ? ui("Closing...") : ui("Force Close")}
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
            {ui("Reset")}
          </Button>
          <Dialog open={resetDialogOpen} onOpenChange={setResetDialogOpen}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{ui("Reset Circuit Breaker?")}</DialogTitle>
                <DialogDescription>
                  {ui('This will delete the circuit breaker state for "')}
                  {breaker.key}
                  {ui('". A new circuit breaker will be created automatically when needed.')}
                </DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button variant="outline" onClick={() => setResetDialogOpen(false)}>
                  {ui("Cancel")}
                </Button>
                <Button onClick={() => deleteMutation.mutate()} disabled={deleteMutation.isPending}>
                  {deleteMutation.isPending ? ui("Resetting...") : ui("Reset")}
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
  const ui = useUiTranslations();
  usePageTitle(ui("Circuit Breakers"), ui("Durable"));
  const { data, isLoading, error } = useCircuitBreakers();

  useDurablePageRows(
    "Circuit breakers",
    (data?.data ?? []).map((b) => ({
      key: b.key,
      state: b.state,
      failure_count: b.failure_count,
      success_count: b.success_count,
      updated_at: b.updated_at,
    })),
    { loading: isLoading, error: !!error },
  );
  if (isLoading) {
    return (
      <div className="operational-page container mx-auto p-6">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <h1 className="text-2xl font-bold">{ui("Circuit Breakers")}</h1>
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
      <div className="operational-page container mx-auto p-6">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <h1 className="text-2xl font-bold">{ui("Circuit Breakers")}</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <ShieldAlert className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">{ui("Failed to Load Circuit Breakers")}</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md">
              {error instanceof Error ? error.message : ui("An error occurred")}
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
    <div className="operational-page container mx-auto p-6">
      <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
        <h1 className="text-2xl font-bold">{ui("Circuit Breakers")}</h1>
      </div>
      <div className="space-y-6">
        {/* Summary Cards */}
        <div className="grid gap-4 md:grid-cols-3">
          <Card>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">{ui("Total")}</CardTitle>
              <Zap className="h-4 w-4 text-muted-foreground" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold">{circuitBreakers.length}</div>
              <p className="text-xs text-muted-foreground">{ui("circuit breakers")}</p>
            </CardContent>
          </Card>
          <Card className={openBreakers.length > 0 ? "border-destructive/50" : ""}>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">{ui("Open")}</CardTitle>
              <ShieldAlert className="h-4 w-4 text-destructive" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-destructive">{openBreakers.length}</div>
              <p className="text-xs text-muted-foreground">{ui("blocking calls")}</p>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">{ui("Closed")}</CardTitle>
              <ShieldCheck className="h-4 w-4 text-success" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-success">{closedBreakers.length}</div>
              <p className="text-xs text-muted-foreground">{ui("allowing calls")}</p>
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
                  <ShieldAlert className="h-5 w-5 text-destructive" />
                  {ui("Open Circuit Breakers")}
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
                  <Clock className="h-5 w-5 text-warning" />
                  {ui("Half-Open Circuit Breakers")}
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
                  <ShieldCheck className="h-5 w-5 text-success" />
                  {ui("Closed Circuit Breakers")}
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
              <h3 className="text-lg font-medium mb-2">{ui("No Circuit Breakers")}</h3>
              <p className="text-sm text-muted-foreground text-center max-w-md">
                {ui(
                  "Circuit breakers are created automatically when protected operations are executed. Check back after running some workflows.",
                )}
              </p>
            </CardContent>
          </Card>
        )}

        {/* Info Card */}
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{ui("About Circuit Breakers")}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground space-y-2">
            <p>
              {ui(
                "Circuit breakers protect against cascading failures when external services (like LLM providers) are unavailable.",
              )}
            </p>
            <ul className="list-disc list-inside space-y-1">
              <li>
                <strong>{ui("Closed:")}</strong>{" "}
                {ui("Normal operation - calls are allowed through")}
              </li>
              <li>
                <strong>{ui("Open:")}</strong>{" "}
                {ui("Service failing - calls are rejected immediately")}
              </li>
              <li>
                <strong>{ui("Half-Open:")}</strong>{" "}
                {ui("Testing recovery - allowing one call to check if service is back")}
              </li>
            </ul>
            <p className="pt-2">
              {ui("Use")} <strong>{ui("Force Open")}</strong>{" "}
              {ui("to proactively block calls when you know a service has issues. Use")}{" "}
              <strong>{ui("Force Close")}</strong>{" "}
              {ui("to immediately allow calls after a service recovers.")}
            </p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
