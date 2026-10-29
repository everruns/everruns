"use client";

import { useState } from "react";
import { useMutation, useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, Play, Terminal, X } from "lucide-react";
import { useOrganizationConnections } from "@/hooks";
import { useUiTranslations } from "@/providers/locale-provider";
import { NetworkPolicyFields } from "@/components/agents/sandbox-policy-editor";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import type { SandboxTemplate, SandboxNetworkPolicy } from "@/lib/api/types";
import {
  applySandboxTemplateBuild,
  cancelSandboxTemplateBuild,
  discardSandboxTemplateBuild,
  listSandboxTemplateBuilds,
  startSandboxTemplateBuild,
} from "@/lib/api/sandbox-template-builds";

const buildLabels: Record<string, string> = {
  queued: "Queued",
  running: "Running",
  ready: "Ready",
  failed: "Failed",
  cancelled: "Cancelled",
  applied: "Saved",
  discarded: "Discarded",
  provisioning: "Starting build compute",
  installing: "Running install script",
  verifying_installation: "Verifying installation",
  snapshotting: "Creating snapshot",
  verifying_snapshot: "Verifying fresh snapshot",
};

export function SandboxTemplateBuildPanel({
  template,
  dirty,
}: {
  template: SandboxTemplate;
  dirty: boolean;
}) {
  const ui = useUiTranslations();
  const confirm = useConfirmation();
  const cache = useQueryClient();
  const spec = template.current_revision.spec;
  const { data: accounts = [] } = useOrganizationConnections();
  const accountName =
    accounts.find((account) => account.id === spec.target.credential?.connection_id)?.name ??
    spec.target.credential?.connection_id ??
    ui("Organization account");
  const options = spec.target.options as Record<string, unknown> | undefined;
  const base =
    typeof options?.snapshot === "string"
      ? options.snapshot
      : options?.desktop
        ? ui("Desktop image")
        : spec.target.provider === "daytona"
          ? `daytona-${options?.size || "small"}`
          : ui("Provider default");
  const supported = spec.target.kind === "managed" && spec.target.provider === "daytona";
  const accountSelected =
    spec.target.credential?.source === "organization" && !!spec.target.credential.connection_id;
  const [script, setScript] = useState("");
  const [verify, setVerify] = useState("");
  const [metadataPath, setMetadataPath] = useState("");
  const [timeout, setTimeout] = useState(600);
  const [network, setNetwork] = useState<SandboxNetworkPolicy>(
    spec.containment?.network ?? { mode: "allow" },
  );
  const queryKey = ["sandbox-template-builds", template.id];
  const builds = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => listSandboxTemplateBuilds(template.id, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => (last.length === 20 ? pages.length * 20 : undefined),
    refetchInterval: (query) =>
      query.state.data?.pages
        .flat()
        .some((b) => b.status === "queued" || b.status === "running" || b.cleanup_pending)
        ? 2000
        : false,
  });
  const start = useMutation({
    mutationFn: () =>
      startSandboxTemplateBuild(template.id, {
        source_revision_id: template.current_revision.id,
        install_script: script,
        verification_command: verify.trim() || undefined,
        metadata_path: metadataPath.trim() || undefined,
        timeout_seconds: timeout,
        build_network: network,
      }),
    onSuccess: () => cache.invalidateQueries({ queryKey }),
  });
  const action = useMutation({
    mutationFn: async ({ id, kind }: { id: string; kind: "cancel" | "discard" | "apply" }) => {
      if (kind === "apply") {
        const updated = await applySandboxTemplateBuild(id);
        cache.setQueryData(["sandbox-template", template.id], updated);
        await cache.invalidateQueries({ queryKey: ["sandbox-templates"] });
      } else if (kind === "cancel") await cancelSandboxTemplateBuild(id);
      else await discardSandboxTemplateBuild(id);
    },
    onSuccess: () => cache.invalidateQueries({ queryKey }),
  });
  const running = builds.data?.pages
    .flat()
    .some((b) => b.status === "queued" || b.status === "running");
  const error = start.error || action.error || builds.error;

  return (
    <div className="space-y-5">
      <Card>
        <CardHeader>
          <CardTitle>{ui("Build snapshot")}</CardTitle>
          <p className="text-sm text-muted-foreground">
            {ui("Run an install script once, then reuse the resulting provider snapshot.")}
          </p>
        </CardHeader>
        <CardContent className="space-y-5">
          <div className="grid gap-3 border bg-muted/40 p-3 sm:grid-cols-2 lg:grid-cols-4">
            <div>
              <p className="text-xs text-muted-foreground">{ui("Provider")}</p>
              <p className="text-sm">{spec.target.provider || spec.target.kind}</p>
            </div>
            <div>
              <p className="text-xs text-muted-foreground">{ui("Base snapshot")}</p>
              <p className="break-all font-mono text-xs">{base}</p>
            </div>
            <div>
              <p className="text-xs text-muted-foreground">{ui("Provider account")}</p>
              <p className="break-words text-sm">{accountName}</p>
            </div>
            <div>
              <p className="text-xs text-muted-foreground">{ui("Source revision")}</p>
              <p className="text-sm">{template.current_revision.revision}</p>
            </div>
          </div>
          {!supported ? (
            <p className="text-sm text-muted-foreground">
              {ui(
                "Snapshot builds currently support Daytona. Other template targets remain configurable.",
              )}
            </p>
          ) : !accountSelected ? (
            <p className="text-sm text-muted-foreground">
              {ui("Select and save a named Organization provider account before building.")}
            </p>
          ) : null}
          {dirty ? (
            <p className="text-sm text-warning">
              {ui("Save your configuration changes before building.")}
            </p>
          ) : null}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label htmlFor="build-install-script">{ui("Install script")}</Label>
              <span className="font-mono text-xs text-muted-foreground">/bin/sh</span>
            </div>
            <Textarea
              id="build-install-script"
              className="min-h-64 font-mono text-xs leading-6"
              spellCheck={false}
              value={script}
              onChange={(e) => setScript(e.target.value)}
              maxLength={32768}
              placeholder={"#!/bin/sh\nset -eu\n\n# Install any tools your sandbox needs.\n"}
              disabled={!!running || start.isPending}
            />
            <p className="text-xs text-muted-foreground">
              {ui(
                "Runs in fresh isolated build compute. Launch bootstrap commands stay separate. Do not put credentials in scripts.",
              )}
            </p>
          </div>
          <details className="border-t pt-4">
            <summary className="cursor-pointer text-sm font-medium">
              {ui("Verification and build access")}
            </summary>
            <div className="mt-4 space-y-4">
              <div className="space-y-2">
                <Label htmlFor="build-verification">{ui("Verification command (optional)")}</Label>
                <Input
                  id="build-verification"
                  className="font-mono text-xs"
                  value={verify}
                  onChange={(e) => setVerify(e.target.value)}
                  maxLength={32768}
                  disabled={!!running}
                />
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "Runs after installation and again on a fresh sandbox from the snapshot. Only what this command checks is verified.",
                  )}
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="build-metadata">{ui("Metadata file (optional)")}</Label>
                <Input
                  id="build-metadata"
                  className="font-mono text-xs"
                  value={metadataPath}
                  onChange={(e) => setMetadataPath(e.target.value)}
                  placeholder="/home/daytona/.everruns/sandbox.md"
                  maxLength={512}
                  disabled={!!running}
                />
                <p className="text-xs text-muted-foreground">
                  {ui("Capture up to 16 KiB of non-secret software notes written by your script.")}
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="build-timeout">{ui("Total build timeout (seconds)")}</Label>
                <Input
                  id="build-timeout"
                  type="number"
                  min={60}
                  max={1800}
                  value={timeout}
                  onChange={(e) => setTimeout(Number(e.target.value))}
                  disabled={!!running}
                />
              </div>
              <div className="space-y-3">
                <p className="text-sm font-medium">{ui("Build network")}</p>
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "Allow package sources during installation without expanding the finished sandbox’s runtime policy.",
                  )}
                </p>
                <div className="grid gap-4 sm:grid-cols-2">
                  <NetworkPolicyFields
                    name="build"
                    network={network}
                    onNetworkChange={setNetwork}
                    disabled={!!running}
                  />
                </div>
              </div>
            </div>
          </details>
          <div className="flex items-start gap-3 border bg-muted/30 p-3">
            <Terminal className="mt-0.5 size-4 shrink-0" />
            <p className="text-xs text-muted-foreground">
              {ui(
                "Build compute and snapshot storage are billed to the selected provider account. Runtime credentials and private workspace data are not attached.",
              )}
            </p>
          </div>
          <Button
            onClick={() => start.mutate()}
            disabled={
              !supported ||
              !accountSelected ||
              dirty ||
              !script.trim() ||
              !!running ||
              start.isPending
            }
            loading={start.isPending}
          >
            <Play className="size-4" />
            {ui("Build snapshot")}
          </Button>
        </CardContent>
      </Card>
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
      {builds.isLoading ? <p className="text-sm text-muted-foreground">{ui("Loading…")}</p> : null}
      {builds.data?.pages.flat().map((build) => {
        const active = build.status === "queued" || build.status === "running";
        const ready = build.status === "ready" && !build.cleanup_pending;
        const current = build.request.source_revision_id === template.current_revision.id;
        return (
          <Card key={build.id}>
            <CardHeader>
              <div className="flex flex-wrap items-center justify-between gap-3">
                <CardTitle>{ui("Build result")}</CardTitle>
                <span className="flex items-center gap-2 text-xs">
                  {active ? (
                    <Loader2 className="size-3.5 animate-spin" />
                  ) : build.status === "ready" || build.status === "applied" ? (
                    <Check className="size-3.5 text-success" />
                  ) : null}
                  {ui(buildLabels[build.status] ?? build.status)}
                </span>
              </div>
            </CardHeader>
            <CardContent className="space-y-4">
              <p className="text-xs text-muted-foreground">
                {new Date(build.created_at).toLocaleString()}
              </p>
              <p className="text-sm">
                {ui("Step")}: {ui(buildLabels[build.step] ?? build.step)}
              </p>
              {build.error ? <p className="text-sm text-destructive">{build.error}</p> : null}
              {build.cleanup_pending ? (
                <p className="text-xs text-warning">
                  {ui("Resource cleanup is pending and will be retried.")}
                </p>
              ) : null}
              {build.snapshot_id ? (
                <p className="break-all font-mono text-xs">{build.snapshot_id}</p>
              ) : null}
              {build.software_notes ? (
                <div className="space-y-2">
                  <h3 className="text-sm font-medium">{ui("Captured software notes")}</h3>
                  <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words border bg-muted/30 p-3 font-mono text-xs">
                    {build.software_notes}
                  </pre>
                  <p className="text-xs text-muted-foreground">
                    {ui(
                      "Author-supplied reference data. These notes do not grant capabilities or change runtime policy.",
                    )}
                  </p>
                </div>
              ) : null}
              <details>
                <summary className="cursor-pointer text-xs">{ui("Build log")}</summary>
                <pre className="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words border bg-muted/30 p-3 font-mono text-xs">
                  {build.logs || ui("Waiting for output…")}
                </pre>
              </details>
              <details>
                <summary className="cursor-pointer text-xs">{ui("Install script")}</summary>
                <pre className="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words border bg-muted/30 p-3 font-mono text-xs">
                  {build.request.install_script}
                </pre>
              </details>
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  disabled={!!running || start.isPending}
                  onClick={() => {
                    setScript(build.request.install_script);
                    setVerify(build.request.verification_command ?? "");
                    setMetadataPath(build.request.metadata_path ?? "");
                    setTimeout(build.request.timeout_seconds ?? 600);
                    setNetwork(
                      build.request.build_network ?? spec.containment?.network ?? { mode: "allow" },
                    );
                    document.getElementById("build-install-script")?.focus();
                  }}
                >
                  {ui("Reuse script")}
                </Button>
                {active ? (
                  <Button
                    variant="outline"
                    disabled={action.isPending || build.cancel_requested}
                    onClick={() => action.mutate({ id: build.id, kind: "cancel" })}
                  >
                    <X className="size-4" />
                    {ui(build.cancel_requested ? "Cancellation requested" : "Cancel build")}
                  </Button>
                ) : null}
                {ready ? (
                  <Button
                    disabled={action.isPending || !current || dirty}
                    onClick={() => action.mutate({ id: build.id, kind: "apply" })}
                  >
                    {ui("Save new revision")}
                  </Button>
                ) : null}
                {!active && build.status !== "applied" && build.status !== "discarded" ? (
                  <Button
                    variant="outline"
                    disabled={action.isPending}
                    onClick={async () => {
                      if (
                        await confirm(
                          ui(
                            "Discard this unused provider snapshot? Applied template revisions are kept.",
                          ),
                        )
                      )
                        action.mutate({ id: build.id, kind: "discard" });
                    }}
                  >
                    {ui("Discard snapshot")}
                  </Button>
                ) : null}
              </div>
              {ready && !current ? (
                <p className="text-xs text-warning">
                  {ui("The source revision changed. Build again from the current template.")}
                </p>
              ) : null}
            </CardContent>
          </Card>
        );
      })}
      {builds.hasNextPage ? (
        <Button
          variant="outline"
          loading={builds.isFetchingNextPage}
          onClick={() => builds.fetchNextPage()}
        >
          {ui("Older builds")}
        </Button>
      ) : null}
    </div>
  );
}
