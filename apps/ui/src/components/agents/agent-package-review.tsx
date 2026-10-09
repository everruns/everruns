"use client";

import { AgentIcon } from "@/components/icons/facet-icons";
import { useState } from "react";
import { FileText, GitCompareArrows, Plug, Settings2 } from "lucide-react";
import type { AgentPackagePreview } from "@/lib/api/agents";
import { Badge } from "@/components/ui/badge";
import { PageMasthead, SectionTabs } from "@/components/layout";
import type { Capability } from "@/lib/api/types";
import { AgentCapabilityList } from "./agent-capability-list";
import { AgentPromptPane } from "./agent-prompt-pane";

function fileSize(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
}

type PackageChange = { path: string; before: unknown; after: unknown };

/** Authored configuration only; validation has not created any resources. */
export function AgentPackageReview({
  preview,
  changes,
  capabilities = [],
}: {
  preview: AgentPackagePreview;
  capabilities?: Capability[];
  changes?: PackageChange[] | null;
}) {
  const [tab, setTab] = useState("agent");
  const files = Object.entries(preview.files);
  const skills = new Set(
    files.flatMap(([path]) => {
      const match = path.match(/^\.agents\/skills\/([^/]+)\/SKILL\.md$/);
      return match ? [match[1]] : [];
    }),
  );
  const ordinaryFiles = files.filter(([path]) => !path.startsWith(".agents/skills/"));
  const servers = Object.entries(preview.mcpServers ?? {});
  const channels = Object.entries(preview.channels ?? {});
  return (
    <section aria-label="Agent preview" className="min-w-0 text-sm">
      <PageMasthead
        className="p-4 sm:p-6"
        icon={<AgentIcon />}
        title={preview.display_name || preview.name}
        badges={
          <>
            <span className="break-all font-mono text-xs text-muted-foreground">
              {preview.name}
            </span>
            <Badge variant="outline">Import preview</Badge>
          </>
        }
        description={preview.description}
      />
      <SectionTabs
        value={tab}
        onValueChange={setTab}
        className="bg-background px-2"
        items={[
          { value: "agent", label: "Agent", icon: <AgentIcon className="size-4" /> },
          {
            value: "files",
            label: "Files",
            icon: <FileText className="size-4" />,
            count: files.length,
          },
          {
            value: "integrations",
            label: "Integrations",
            icon: <Plug className="size-4" />,
            count: channels.length,
          },
          { value: "settings", label: "Settings", icon: <Settings2 className="size-4" /> },
          ...(changes
            ? [
                {
                  value: "changes",
                  label: "Changes",
                  icon: <GitCompareArrows className="size-4" />,
                  count: changes.length,
                },
              ]
            : []),
        ]}
      />
      {tab === "agent" && (
        <div className="grid min-w-0 lg:grid-cols-[minmax(0,1fr)_280px]">
          <AgentPromptPane
            value={preview.instructions}
            editing={false}
            skipHtml
            className="lg:border-r"
          />
          <aside
            aria-label="Agent configuration"
            className="min-w-0 border-t bg-muted/30 p-4 lg:border-t-0"
          >
            <dl className="grid gap-4">
              <div>
                <dt className="mb-1.5 text-xs font-medium text-muted-foreground">Harness</dt>
                <dd>{preview.harness || "Destination default harness"}</dd>
              </div>
              <div>
                <dt className="mb-1.5 text-xs font-medium text-muted-foreground">Default model</dt>
                <dd className="break-words">
                  {preview.model
                    ? `${preview.model.provider} / ${preview.model.model}`
                    : "Destination default model"}
                </dd>
              </div>
            </dl>
            <div className="mt-4">
              <h4 className="text-xs font-medium text-muted-foreground">
                Capabilities ({Object.keys(preview.capabilities).length})
              </h4>
              <div className="mt-2">
                <AgentCapabilityList
                  references={Object.keys(preview.capabilities)}
                  capabilities={capabilities}
                />
              </div>
              {Object.entries(preview.capabilities)
                .filter(([, config]) => config && Object.keys(config as object).length > 0)
                .map(([name, config]) => (
                  <details key={name} className="mt-2">
                    <summary>{name} settings</summary>
                    <pre className="overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">
                      {JSON.stringify(config, null, 2)}
                    </pre>
                  </details>
                ))}
            </div>

            <div className="mt-4">
              <p className="mb-1.5 text-xs font-medium text-muted-foreground">Tags</p>
              <div className="flex flex-wrap gap-1">
                {preview.tags?.length ? (
                  preview.tags.map((tag) => (
                    <Badge key={tag} variant="outline">
                      {tag}
                    </Badge>
                  ))
                ) : (
                  <span className="text-muted-foreground">No tags</span>
                )}
              </div>
            </div>
          </aside>
        </div>
      )}
      {tab === "files" && (
        <div className="p-4 sm:p-6">
          <div>
            <h4 className="font-medium">
              Files ({ordinaryFiles.length}) · Skills ({skills.size})
            </h4>
            <p className="mt-1 text-xs text-muted-foreground">
              Relative paths in the new session’s file tree. Skill support files are included.
            </p>
            {!!skills.size && <p className="mt-2">Skills: {Array.from(skills).join(", ")}</p>}
            {files.length ? (
              <ul className="mt-2 max-h-48 overflow-auto divide-y rounded border">
                {files.map(([path, file]) => (
                  <li
                    key={path}
                    className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
                  >
                    <span className="break-all font-mono text-xs">{path}</span>
                    <span className="text-xs text-muted-foreground">
                      {fileSize(file.bytes)} · {file.is_readonly ? "Read-only" : "Writable"}
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-2 text-muted-foreground">No starting files</p>
            )}
          </div>
        </div>
      )}
      {tab === "integrations" && (
        <div className="p-4 sm:p-6">
          <div>
            <h4 className="font-medium">Channels ({channels.length})</h4>
            {channels.length ? (
              <ul className="mt-2 grid gap-2">
                {channels.map(([name, channel]) => (
                  <li key={name}>
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{name}</span>
                      <span>{channel.type}</span>
                      <Badge variant="secondary">
                        {channel.enabled ? "Draft requested" : "Disabled for new channels"}
                      </Badge>
                    </div>
                    <details className="mt-1">
                      <summary className="text-muted-foreground">Channel settings</summary>
                      <pre className="overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">
                        {JSON.stringify(channel.config, null, 2)}
                      </pre>
                    </details>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-1 text-muted-foreground">None declared</p>
            )}
            <p className="mt-2 text-xs text-muted-foreground">
              Publication and credentials stay at the destination. Updates preserve existing channel
              activation and omitted channels.
            </p>
          </div>
        </div>
      )}
      {tab === "settings" && (
        <div className="grid gap-6 p-4 sm:p-6">
          <dl className="grid gap-3 sm:grid-cols-2">
            <div>
              <dt className="text-muted-foreground">Iteration limit</dt>
              <dd>{preview.max_iterations ?? "Runtime default"}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Parallel tool calls</dt>
              <dd>
                {preview.parallel_tool_calls == null
                  ? "Runtime default"
                  : preview.parallel_tool_calls
                    ? "Enabled"
                    : "Disabled"}
              </dd>
            </div>
          </dl>
          <div>
            <h4 className="font-medium">MCP servers ({servers.length})</h4>
            {servers.length ? (
              <ul className="mt-2 grid gap-2">
                {servers.map(([name, server]) => (
                  <li key={name}>
                    <span className="font-medium">{name}</span>
                    <span className="ml-2 break-all text-muted-foreground">
                      {server.use || server.url || server.command || server.type}
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-1 text-muted-foreground">None declared</p>
            )}
          </div>
          {!!preview.tools?.length && (
            <div className="mt-4">
              <h4 className="text-xs font-medium text-muted-foreground">
                Custom tools ({preview.tools.length})
              </h4>
              {preview.tools.map((tool) => (
                <details key={tool.name} className="mt-2">
                  <summary>{tool.name}</summary>
                  <p className="mt-1">{tool.description}</p>
                  <pre className="overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">
                    {JSON.stringify(tool.parameters, null, 2)}
                  </pre>
                </details>
              ))}
              <p className="mt-2 text-xs text-muted-foreground">
                Executable implementations must be bound by the destination.
              </p>
            </div>
          )}
          {[
            ["Network access", preview.network_access],
            ["Environments", preview.environments],
            ["Introduction", preview.intro_markdown],
            ["Short description", preview.short_description],
            ["Conversation starters", preview.starters?.length ? preview.starters : undefined],
          ]
            .filter(([, value]) => value != null)
            .map(([label, value]) => (
              <details key={String(label)}>
                <summary className="font-medium">{String(label)}</summary>
                <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-2 text-xs">
                  {typeof value === "string" ? value : JSON.stringify(value, null, 2)}
                </pre>
              </details>
            ))}
        </div>
      )}
      {tab === "changes" && changes && (
        <div className="p-4 sm:p-6">
          <PackageChangeReview changes={changes} />
        </div>
      )}
    </section>
  );
}

function valueText(value: unknown): string {
  if (value == null) return "Not set";
  if (typeof value === "string") return value;
  if (typeof value === "object" && "bytes" in value && "is_readonly" in value) {
    const file = value as { bytes: number; is_readonly: boolean };
    return `${fileSize(file.bytes)} · ${file.is_readonly ? "Read-only" : "Writable"}`;
  }
  return JSON.stringify(value, null, 2);
}

export function PackageChangeReview({
  changes,
}: {
  changes: { path: string; before: unknown; after: unknown }[];
}) {
  return (
    <section aria-label="Changes to existing agent" className="grid gap-2 text-sm">
      <h3 className="font-medium">
        {changes.length
          ? `${changes.length} ${changes.length === 1 ? "change" : "changes"}`
          : "No changes to apply"}
      </h3>
      {changes.map((change) => {
        const path = change.path
          .slice(1)
          .split("/")
          .map((part) => part.replaceAll("~1", "/").replaceAll("~0", "~"))
          .join(" › ");
        return (
          <details key={change.path} className="rounded border p-3">
            <summary className="cursor-pointer break-all">
              <span className="font-medium">{path}</span>
              <Badge variant="secondary" className="ml-2">
                {change.before == null ? "Added" : change.after == null ? "Removed" : "Updated"}
              </Badge>
            </summary>
            <div className="mt-3 grid gap-2 sm:grid-cols-2">
              {(["before", "after"] as const).map((side) => (
                <div key={side}>
                  <h4 className="mb-1 text-xs font-medium text-muted-foreground">
                    {side === "before" ? "Current" : "Imported"}
                  </h4>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-2 font-sans text-xs">
                    {valueText(change[side])}
                  </pre>
                </div>
              ))}
            </div>
            {change.path.startsWith("/files/") && (
              <p className="mt-2 text-xs text-muted-foreground">
                {change.before == null ? (
                  "New starting file."
                ) : change.after == null ? (
                  "Starting file removed."
                ) : (
                  <>
                    {(change.before as { sha256: string }).sha256 !==
                      (change.after as { sha256: string }).sha256 && "Contents changed. "}
                    {(change.before as { is_readonly: boolean }).is_readonly !==
                      (change.after as { is_readonly: boolean }).is_readonly &&
                      "Permissions changed. "}
                  </>
                )}{" "}
                File bodies are not shown in the diff.
              </p>
            )}
          </details>
        );
      })}
    </section>
  );
}
