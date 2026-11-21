"use client";

import dynamic from "next/dynamic";
import { useId, useState } from "react";
import type { AgentViewLayout } from "@everruns/generative-ui";
import "@everruns/generative-ui/styles.css";
import { Button, LinkButton } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { type useAgentViewSettings, useReplaceAgentView } from "@/hooks/use-agent-view-settings";
import type { AgentViewDefinition, AgentViewSettings } from "@/lib/api/agent-views";
import { ApiError } from "@/lib/api/client";
import { useUiTranslations } from "@/providers/locale-provider";

const ViewSurface = dynamic(
  () => import("@everruns/generative-ui").then((module) => module.AgentViewSurface),
  { ssr: false },
);

const EXAMPLES = {
  a2ui: JSON.stringify(
    [
      {
        version: "1.0",
        createSurface: { surfaceId: "view", catalogId: "copilotkit/standard@1" },
      },
      {
        version: "1.0",
        updateComponents: {
          surfaceId: "view",
          components: [{ id: "root", component: "Text", text: "No changes need review." }],
        },
      },
    ],
    null,
    2,
  ),
  html: "<section>\n  <h2>Changes</h2>\n  <p>No changes need review.</p>\n</section>",
};

function fromSaved(settings: AgentViewSettings) {
  const view = settings.view;
  const layout = view?.layout;
  return {
    revision: settings.revision,
    title: view?.title ?? "",
    format: layout?.format ?? "a2ui",
    source: {
      a2ui: layout?.format === "a2ui" ? JSON.stringify(layout.operations, null, 2) : "",
      html: layout?.format === "html" ? layout.source : "",
    },
    allowPersonalization: view?.allow_personalization ?? false,
    dataFields: view?.data_fields.join(", ") ?? "",
    allowAgentLayout: view?.allow_agent_layout ?? false,
  };
}

export function AgentViewSettingsEditor({
  agentId,
  query,
  readOnly,
}: {
  agentId: string;
  query: ReturnType<typeof useAgentViewSettings>;
  readOnly: boolean;
}) {
  const ui = useUiTranslations();
  if (query.isError && !query.data) {
    return (
      <div className="space-y-2" role="alert">
        <p>{ui("Could not load view settings.")}</p>
        <Button variant="outline" onClick={() => void query.refetch()}>
          {ui("Retry")}
        </Button>
      </div>
    );
  }
  if (!query.data) return <p role="status">{ui("Loading view settings…")}</p>;
  return (
    <ViewForm
      agentId={agentId}
      settings={query.data}
      reload={async () => (await query.refetch()).data}
      readOnly={readOnly}
    />
  );
}

function ViewForm({
  agentId,
  settings,
  reload,
  readOnly,
}: {
  agentId: string;
  settings: AgentViewSettings;
  reload: () => Promise<AgentViewSettings | undefined>;
  readOnly: boolean;
}) {
  const ui = useUiTranslations();
  const id = useId();
  const confirm = useConfirmation();
  const update = useReplaceAgentView(agentId);
  const [draft, setDraft] = useState(() => fromSaved(settings));
  const [preview, setPreview] = useState<AgentViewDefinition | null>(settings.view);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const disabled = readOnly || update.isPending;
  const stale = update.error instanceof ApiError && update.error.status === 409;

  function change(next: Partial<typeof draft>) {
    setDraft({ ...draft, ...next });
    setError("");
    setNotice("");
  }

  function makeView(): AgentViewDefinition | null {
    const title = draft.title.trim();
    const source = draft.source[draft.format].trim();
    if (!title || !source) {
      setError(ui("Enter a title and layout source."));
      return null;
    }
    let layout: AgentViewLayout;
    if (draft.format === "a2ui") {
      try {
        const operations: unknown = JSON.parse(source);
        if (!Array.isArray(operations) || operations.length === 0) throw new Error();
        layout = { format: "a2ui", operations };
      } catch {
        setError(ui("Enter a JSON array of A2UI operations."));
        return null;
      }
    } else {
      layout = { format: "html", source };
    }
    setError("");
    return {
      title,
      layout,
      allow_personalization: draft.allowPersonalization,
      allow_agent_layout: draft.allowAgentLayout,
      data_fields: draft.dataFields
        .split(",")
        .map((field) => field.trim())
        .filter(Boolean),
    };
  }

  function save(view: AgentViewDefinition | null) {
    setNotice("");
    setError("");
    update.mutate(
      { revision: draft.revision, view },
      {
        onSuccess: (saved) => {
          setDraft(fromSaved(saved));
          setPreview(saved.view);
          setNotice(ui("View settings saved."));
        },
        onError: (error) => setError(error.message),
      },
    );
  }

  return (
    <div className="space-y-5">
      <section aria-labelledby={`${id}-layout`} className="space-y-3">
        <h3 id={`${id}-layout`} className="text-sm font-semibold">
          {ui("Layout")}
        </h3>
        <div className="space-y-1.5">
          <Label htmlFor={`${id}-title`}>{ui("View title")}</Label>
          <Input
            id={`${id}-title`}
            value={draft.title}
            maxLength={120}
            disabled={disabled}
            onChange={(event) => change({ title: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor={`${id}-format`}>{ui("Format")}</Label>
          <select
            id={`${id}-format`}
            className="block h-9 w-full border bg-background px-2 text-sm disabled:opacity-60"
            value={draft.format}
            disabled={disabled}
            onChange={(event) =>
              change({ format: event.target.value === "html" ? "html" : "a2ui" })
            }
          >
            <option value="a2ui">{ui("Generated UI (A2UI)")}</option>
            <option value="html">{ui("Custom HTML")}</option>
          </select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor={`${id}-source`}>
            {ui(draft.format === "a2ui" ? "A2UI operations" : "HTML markup")}
          </Label>
          <Textarea
            id={`${id}-source`}
            className="min-h-36 font-mono text-xs"
            value={draft.source[draft.format]}
            disabled={disabled}
            spellCheck={false}
            onChange={(event) =>
              change({
                source: { ...draft.source, [draft.format]: event.target.value },
              })
            }
          />
          <p className="text-xs text-muted-foreground">
            {draft.format === "a2ui"
              ? ui("Use A2UI 1.0 operations for a single surface with the standard catalog.")
              : ui("HTML supports static markup and styles.")}
          </p>
        </div>
        {!readOnly && (
          <div className="flex flex-wrap gap-2">
            {!draft.source[draft.format] && (
              <Button
                variant="outline"
                disabled={disabled}
                onClick={() =>
                  change({
                    source: { ...draft.source, [draft.format]: EXAMPLES[draft.format] },
                  })
                }
              >
                {ui("Use example")}
              </Button>
            )}
            <Button
              variant="outline"
              disabled={disabled}
              onClick={() => {
                const view = makeView();
                if (view) setPreview(view);
              }}
            >
              {ui("Preview draft")}
            </Button>
          </div>
        )}
      </section>
      <section aria-labelledby={`${id}-publishing`} className="space-y-3 border-t pt-4">
        <h3 id={`${id}-publishing`} className="text-sm font-semibold">
          {ui("Publishing")}
        </h3>
        <div className="space-y-1.5">
          <Label htmlFor={`${id}-fields`}>{ui("Data fields")}</Label>
          <Input
            id={`${id}-fields`}
            value={draft.dataFields}
            disabled={disabled}
            placeholder="checked, status"
            onChange={(event) => change({ dataFields: event.target.value })}
          />
          <p className="text-xs text-muted-foreground">
            {ui(
              "Enter comma-separated names and tell the agent what to publish in its instructions. Bind them in A2UI or use {{field}} in HTML text.",
            )}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id={`${id}-generated`}
            checked={draft.allowAgentLayout}
            disabled={disabled}
            onCheckedChange={(allowAgentLayout) => change({ allowAgentLayout })}
          />
          <Label htmlFor={`${id}-generated`}>{ui("Allow the agent to publish a new layout")}</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          {ui(
            "Add the Published views capability to give the agent the publish_view tool. Share the view with a Work Chat audience from that channel’s settings.",
          )}
        </p>
      </section>
      <section aria-labelledby={`${id}-personalization`} className="space-y-3 border-t pt-4">
        <h3 id={`${id}-personalization`} className="text-sm font-semibold">
          {ui("Personalization")}
        </h3>
        <div className="flex items-center gap-2">
          <Checkbox
            id={`${id}-personal`}
            checked={draft.allowPersonalization}
            disabled={disabled}
            onCheckedChange={(allowPersonalization) => change({ allowPersonalization })}
          />
          <Label htmlFor={`${id}-personal`}>{ui("Allow personal layouts")}</Label>
        </div>
      </section>
      {preview && (
        <section aria-labelledby={`${id}-preview`} className="space-y-3 border-t pt-4">
          <h3 id={`${id}-preview`} className="text-sm font-semibold">
            {ui("Preview")}
          </h3>
          <ViewSurface title={preview.title} layout={preview.layout} />
        </section>
      )}
      {settings.view && (
        <LinkButton variant="outline" href={"/chats?view=" + encodeURIComponent(agentId)}>
          {ui("Open in Platform Chat")}
        </LinkButton>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-sm text-muted-foreground">
          {notice}
        </p>
      )}
      {!readOnly && (
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={disabled || stale}
            onClick={() => {
              const view = makeView();
              if (view) save(view);
            }}
          >
            {ui(update.isPending ? "Saving…" : "Save view")}
          </Button>
          {settings.view && (
            <Button
              variant="outline"
              disabled={disabled || stale}
              onClick={async () => {
                if (await confirm(ui("Remove this agent’s view?"))) save(null);
              }}
            >
              {ui("Remove view")}
            </Button>
          )}
          {stale && (
            <Button
              variant="outline"
              onClick={async () => {
                const latest = await reload();
                if (latest) {
                  setDraft(fromSaved(latest));
                  setPreview(latest.view);
                  setError("");
                  update.reset();
                }
              }}
            >
              {ui("Reload saved view")}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
