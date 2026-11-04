"use client";
import { useState } from "react";
import { Plus, MoreHorizontal, ArrowUpRight } from "lucide-react";
import Link from "next/link";
import { usePageTitle } from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useTracingDestinations, useRefreshTracing } from "@/hooks/use-tracing";
import {
  deleteTracingDestination,
  saveTracingDestination,
  testTracingDestination,
  type TracingDestination,
} from "@/lib/api/tracing";
import { TracingDestinationDialog } from "./destination-dialog";
import { TracingRoutingForm } from "./routing-form";
import { useTracingDestinationRows } from "./use-tracing-page-context";
import { HistorySheet } from "@/components/entity-actions/history-sheet";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableHead,
  TableRow,
  TableBody,
  TableCell,
} from "@/components/ui/table";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuPositioner,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
export function TracingSettings() {
  const { currentOrg } = useOrg();
  return <OrganizationTracing key={currentOrg?.public_id} />;
}
function OrganizationTracing() {
  const ui = useUiTranslations();
  usePageTitle(ui("Tracing"), ui("Settings"));
  const { currentOrg, hasRole } = useOrg();
  const query = useTracingDestinations();
  useTracingDestinationRows(query.data ?? [], { loading: query.isLoading, error: !!query.error });
  const refresh = useRefreshTracing();
  const canManage = hasRole("admin");
  const [editor, setEditor] = useState<TracingDestination | "new" | null>(null);
  const [history, setHistory] = useState<TracingDestination | null>(null);
  const [routing, setRouting] = useState(false);
  const [removing, setRemoving] = useState<TracingDestination | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  async function action(operation: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await operation();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : ui("Tracing operation failed"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h2 className="text-xl font-semibold">{ui("Tracing")}</h2>
          <p className="text-sm text-muted-foreground">
            {ui("Send agent traces to your Braintrust projects and OTLP collectors.")}
          </p>
        </div>
        {canManage && (
          <Button variant="accent" onClick={() => setEditor("new")}>
            <Plus className="size-4" />
            {ui("Add destination")}
          </Button>
        )}
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {message && (
        <p role="status" className="text-sm">
          {message}
        </p>
      )}
      {query.isLoading ? (
        <p className="text-sm text-muted-foreground">{ui("Loading tracing…")}</p>
      ) : query.error ? (
        <div role="alert">
          <p>{ui("Could not load tracing")}</p>
          <Button variant="outline" onClick={() => void query.refetch()}>
            {ui("Retry")}
          </Button>
        </div>
      ) : query.data?.length === 0 ? (
        <Card className="p-6">
          <h3 className="text-sm font-medium">{ui("No tracing destinations")}</h3>
          <p className="mt-2 text-sm text-muted-foreground">
            {ui(
              "Connect Braintrust or an OpenTelemetry collector, then choose which agents export traces.",
            )}
          </p>
        </Card>
      ) : (
        <Card className="gap-0 overflow-hidden p-0">
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{ui("Destination")}</TableHead>
                  <TableHead>{ui("Routing")}</TableHead>
                  <TableHead>{ui("Content")}</TableHead>
                  <TableHead>{ui("Delivery")}</TableHead>
                  <TableHead className="w-12" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {query.data?.map((d) => (
                  <TableRow key={d.id}>
                    <TableCell className="py-4">
                      <button
                        className="text-left text-sm font-medium enabled:hover:underline"
                        disabled={!canManage}
                        onClick={() => canManage && setEditor(d)}
                      >
                        {d.config.name}
                      </button>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {d.config.provider === "braintrust" ? "Braintrust" : "OpenTelemetry / OTLP"}
                      </p>
                    </TableCell>
                    <TableCell className="text-xs">
                      {d.organization_default && <p>{ui("Organization default")}</p>}
                      {d.agent_selections > 0 && (
                        <p>
                          {ui("Agent selections: {value0}", { value0: String(d.agent_selections) })}
                        </p>
                      )}
                      {!d.organization_default && d.agent_selections === 0 && ui("Unassigned")}
                    </TableCell>
                    <TableCell className="text-xs">
                      {ui(d.config.capture_content ? "Messages and tools" : "Metadata only")}
                    </TableCell>
                    <TableCell>
                      <span className="flex items-center gap-1.5 text-xs">
                        <span
                          className={`size-1.5 rounded-full ${!d.config.enabled ? "bg-muted-foreground" : d.last_error ? "bg-warning" : d.last_accepted_at ? "bg-success" : "bg-muted-foreground"}`}
                        />
                        {ui(
                          !d.config.enabled
                            ? "Paused"
                            : d.last_error
                              ? "Delivery failing"
                              : d.last_accepted_at
                                ? "Export accepted"
                                : "No exports yet",
                        )}
                      </span>
                      {d.last_error && (
                        <p className="mt-1 max-w-xs text-xs text-muted-foreground">
                          {d.last_error}
                        </p>
                      )}
                      {d.dropped_events > 0 && (
                        <p className="mt-1 text-xs text-warning">
                          {ui("Dropped events: {value0}", { value0: String(d.dropped_events) })}
                        </p>
                      )}
                    </TableCell>
                    <TableCell>
                      {canManage && (
                        <DropdownMenu>
                          <DropdownMenuTrigger
                            render={
                              <Button
                                variant="ghost"
                                size="icon"
                                aria-label={`${ui("Manage")} ${d.config.name}`}
                                disabled={busy}
                              />
                            }
                          >
                            <MoreHorizontal className="size-4" />
                          </DropdownMenuTrigger>
                          <DropdownMenuPositioner align="end">
                            <DropdownMenuContent>
                              <DropdownMenuItem onClick={() => setEditor(d)}>
                                {ui("Edit destination")}
                              </DropdownMenuItem>
                              <DropdownMenuItem
                                onClick={() =>
                                  void action(async () => {
                                    const result = await testTracingDestination(d.id);
                                    setMessage(
                                      result.accepted
                                        ? ui("Synthetic trace accepted by receiver")
                                        : ui(
                                            "Test export failed; review destination delivery status",
                                          ),
                                    );
                                  })
                                }
                              >
                                {ui("Send test trace")}
                              </DropdownMenuItem>
                              <DropdownMenuItem
                                onClick={() =>
                                  void action(() =>
                                    saveTracingDestination(
                                      {
                                        config: { ...d.config, enabled: !d.config.enabled },
                                        revision: d.revision,
                                      },
                                      d.id,
                                    ),
                                  )
                                }
                              >
                                {ui(d.config.enabled ? "Pause destination" : "Resume destination")}
                              </DropdownMenuItem>
                              <DropdownMenuItem onClick={() => setHistory(d)}>
                                {ui("History")}
                              </DropdownMenuItem>
                              <DropdownMenuItem onClick={() => setRemoving(d)}>
                                {ui("Delete destination")}
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenuPositioner>
                        </DropdownMenu>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </Card>
      )}
      <div className="flex items-center justify-between gap-4 border-t pt-4">
        <div>
          <h3 className="text-sm font-medium">{ui("Organization defaults")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {ui(
              "Agents inherit these destinations unless they customize routing. Personal Chat is excluded.",
            )}
          </p>
        </div>
        <Button variant="outline" onClick={() => setRouting((v) => !v)}>
          {ui(routing ? "Close" : canManage ? "Edit defaults" : "View defaults")}
        </Button>
      </div>
      {routing && (
        <Card className="p-5">
          <TracingRoutingForm key={currentOrg?.public_id} />
        </Card>
      )}
      <p className="text-xs text-muted-foreground">
        {ui(
          "Platform telemetry is configured separately. Exports are best effort; delivery failures do not stop agents.",
        )}
      </p>
      {history && (
        <HistorySheet
          entityRef={history.id}
          kind="tracing_destination"
          entityName={history.config.name}
          canRestore={canManage && !busy}
          open
          onOpenChange={(open) => {
            if (!open) {
              setHistory(null);
              void refresh();
            }
          }}
        />
      )}
      {editor && (
        <TracingDestinationDialog
          key={`${currentOrg?.public_id}:${editor === "new" ? "new" : editor.id}`}
          destination={editor === "new" ? undefined : editor}
          onClose={() => setEditor(null)}
          onCreated={() => {
            setMessage(
              ui(
                "Destination created. Choose defaults or assign it from an agent's Tracing settings.",
              ),
            );
            setRouting(true);
          }}
        />
      )}
      <Link
        href="/settings/health"
        className="inline-flex items-center gap-1 text-xs text-muted-foreground underline"
      >
        {ui("Organization health")}
        <ArrowUpRight className="size-3" />
      </Link>
      {removing && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setRemoving(null);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{ui("Delete destination")}</DialogTitle>
              <DialogDescription>
                {ui(
                  "Remove this destination from all routing policies first. Existing exports at the provider are not deleted.",
                )}
              </DialogDescription>
            </DialogHeader>
            <p className="text-sm font-medium">{removing.config.name}</p>
            <DialogFooter>
              <Button variant="outline" onClick={() => setRemoving(null)}>
                {ui("Cancel")}
              </Button>
              <Button
                variant="destructive"
                disabled={busy}
                onClick={() =>
                  void action(async () => {
                    await deleteTracingDestination(removing.id, removing.revision);
                    setRemoving(null);
                  })
                }
              >
                {ui("Delete destination")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
