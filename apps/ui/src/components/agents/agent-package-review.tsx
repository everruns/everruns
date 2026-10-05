import type { AgentPackagePreview } from "@/lib/api/agents";
import { Badge } from "@/components/ui/badge";

function fileSize(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
}

/** Authored configuration only; validation has not created any resources. */
export function AgentPackageReview({ preview }: { preview: AgentPackagePreview }) {
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
    <section aria-label="Agent preview" className="grid gap-4 rounded-lg border p-4 text-sm">
      <div>
        <h3 className="text-base font-semibold">{preview.display_name || preview.name}</h3>
        <p className="text-muted-foreground">{preview.name}</p>
        {preview.description && <p className="mt-2">{preview.description}</p>}
        {!!preview.tags?.length && (
          <div className="mt-2 flex flex-wrap gap-1">
            {preview.tags.map((tag) => (
              <Badge key={tag} variant="secondary">
                {tag}
              </Badge>
            ))}
          </div>
        )}
      </div>
      <dl className="grid grid-cols-2 gap-3">
        <div>
          <dt className="text-muted-foreground">Model</dt>
          <dd>
            {preview.model
              ? `${preview.model.provider} / ${preview.model.model}`
              : "Destination default model"}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Harness</dt>
          <dd>{preview.harness || "Destination default harness"}</dd>
        </div>
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
        <h4 className="font-medium">Instructions</h4>
        <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-3 font-sans text-sm">
          {preview.instructions}
        </pre>
      </div>
      <div>
        <h4 className="font-medium">Capabilities ({Object.keys(preview.capabilities).length})</h4>
        <div className="mt-2 flex flex-wrap gap-1">
          {Object.keys(preview.capabilities).map((name) => (
            <Badge key={name} variant="secondary">
              {name}
            </Badge>
          ))}
          {!Object.keys(preview.capabilities).length && (
            <span className="text-muted-foreground">None declared</span>
          )}
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
        <div>
          <h4 className="font-medium">Custom tools ({preview.tools.length})</h4>
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
