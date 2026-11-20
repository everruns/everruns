"use client";
import { Label } from "@/components/ui/label";

// The agent page's narrow config column. Primary settings (harness,
// capabilities, model, communication, tags) are always visible; everything set once and
// rarely changed is one "More" row each that shows its current value and
// opens a side sheet. Editable values wear a bordered control; read-only facts
// (Updated, and every value on an archived agent) are plain muted text.
//
// The controls are live in view mode too: changing one puts the page into
// edit mode with that change pending, so nothing is saved until Save.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { Pencil } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TagInput } from "@/components/ui/tag-input";
import { HarnessSelect } from "@/components/harness/harness-select";
import { ModelPicker } from "@/components/models/model-picker";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useModelPresets } from "@/hooks/use-model-presets";
import { HelperModelsSection } from "@/components/agents/helper-models-section";
import { useOrganization } from "@/hooks/use-organizations";
import { RESEARCH_MODEL_OPTION, agentModelId } from "@/lib/agent-model-role";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import { CommunicationSelect } from "@/components/agents/communication-select";
import { getCommunicationOption, normalizeCommunication } from "@/lib/agent-communication";
import type { AgentDraft } from "@/components/agents/use-agent-draft";
import type { Agent, Capability, ModelWithProvider } from "@/lib/api/types";
import { AgentCapabilityList } from "./agent-capability-list";
import { CapabilityEditorContext } from "./capability-editor-context";
import { normalizeTags } from "@/lib/tags";
import { MoreSettingsNav, type MoreSettingsRow } from "@/components/workspace/more-settings-nav";
import { cn } from "@/lib/utils";
import { ChatChangeMark, useChatChanges } from "@/components/chat-mode/chat-change-mark";

/** Picker value for "Organization defaults" (no model preset). */
const ORG_DEFAULTS = "__org_defaults__";

export type AgentMoreRow = MoreSettingsRow;

interface AgentConfigColumnProps {
  agent: Agent;
  draft: AgentDraft;
  editing: boolean;
  readOnly: boolean;
  allCapabilities: Capability[];
  defaultModel?: ModelWithProvider;
  /** The model the organization research model resolves to now. */
  researchModel?: ModelWithProvider;
  onStartEdit: () => void;
  moreRows: AgentMoreRow[];
  onOpenRow: (id: string) => void;
  /** An external agent's endpoint owns harness, capabilities, model and communication. */
  external?: boolean;
  className?: string;
}

export function AgentConfigColumn({
  agent,
  draft,
  editing,
  readOnly,
  allCapabilities,
  defaultModel,
  researchModel,
  onStartEdit,
  moreRows,
  onOpenRow,
  external = false,
  className,
}: AgentConfigColumnProps) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const { data: organization } = useOrganization();
  const { data: modelPresets = [] } = useModelPresets();
  const chatChanges = useChatChanges();
  // Any control change starts (or continues) the page-level edit.
  const edit = <T,>(apply: (value: T) => void) => {
    return (value: T) => {
      apply(value);
      if (!editing) onStartEdit();
    };
  };

  return (
    <aside
      aria-label={ui("Agent configuration")}
      className={cn("flex min-w-0 flex-col bg-muted/30", className)}
    >
      <div className="flex flex-col gap-4 border-b p-4">
        {!external && (
          <>
            <ChatChangeMark field="harness_id" className="flex flex-col gap-1.5">
              <Label className="text-xs font-medium text-muted-foreground" htmlFor="harness">
                {ui("Harness")}
              </Label>
              {readOnly ? (
                <span className="text-[13px] text-muted-foreground">
                  {agent.effective_harness?.display_name ||
                    agent.effective_harness?.name ||
                    ui("Default")}
                </span>
              ) : (
                <HarnessSelect
                  id="harness"
                  value={draft.fields.harness_id}
                  onValueChange={edit((value: string) => draft.setField("harness_id", value))}
                  placeholder={ui("Select a harness")}
                  className="w-full bg-background"
                />
              )}
              {draft.errors.harness_id && (
                <p className="text-xs text-destructive">{draft.errors.harness_id}</p>
              )}
            </ChatChangeMark>

            <ChatChangeMark field="capabilities" className="flex flex-col gap-1.5">
              {editing ? (
                <CapabilityEditorContext.Provider
                  value={{
                    modelId: agentModelId(draft.fields.default_model_id, organization),
                    agentId: agent.id,
                  }}
                >
                  <CapabilitySelector
                    capabilities={allCapabilities}
                    selected={draft.capabilities}
                    onChange={draft.setCapabilities}
                    label={ui("Capabilities")}
                    compact
                  />
                </CapabilityEditorContext.Provider>
              ) : (
                <>
                  <div className="flex items-center gap-2">
                    <span className="flex-1 text-xs font-medium text-muted-foreground">
                      {ui("Capabilities")}
                    </span>
                    {!readOnly && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-6 px-1.5 text-xs"
                        onClick={onStartEdit}
                        aria-label={ui("Edit capabilities")}
                      >
                        <Pencil className="size-3" />
                        {ui("Edit")}
                      </Button>
                    )}
                  </div>
                  <AgentCapabilityList
                    references={draft.capabilities.map((config) => config.ref)}
                    capabilities={allCapabilities}
                  />
                </>
              )}
            </ChatChangeMark>

            <ChatChangeMark field="default_model_id" className="flex flex-col gap-1.5">
              <Label className="text-xs font-medium text-muted-foreground" htmlFor="default-model">
                {ui("Default model")}
              </Label>
              {readOnly ? (
                <span className="text-[13px] text-muted-foreground">
                  {agent.model_role === "research"
                    ? [ui("Organization research model"), defaultModel?.display_name]
                        .filter(Boolean)
                        .join(" · ")
                    : (defaultModel?.display_name ?? ui("Organization default"))}
                </span>
              ) : (
                <ModelPicker
                  id="default-model"
                  additionalOptions={[
                    {
                      value: RESEARCH_MODEL_OPTION,
                      label: [ui("Organization research model"), researchModel?.display_name]
                        .filter(Boolean)
                        .join(" · "),
                    },
                  ]}
                  value={draft.fields.default_model_id}
                  onChange={edit((value: string) => draft.setField("default_model_id", value))}
                  placeholder={ui("Use default model")}
                  className="w-full bg-background"
                />
              )}
            </ChatChangeMark>

            <ChatChangeMark field="model_preset" className="flex flex-col gap-1.5">
              <Label className="text-xs font-medium text-muted-foreground" htmlFor="model-preset">
                {ui("Models from")}
              </Label>
              {readOnly ? (
                <span className="text-[13px] text-muted-foreground">
                  {modelPresets.find((preset) => preset.id === agent.model_preset)?.name ??
                    agent.model_preset ??
                    ui("Organization defaults")}
                </span>
              ) : (
                <Select
                  value={draft.fields.model_preset || ORG_DEFAULTS}
                  onValueChange={edit((value: string | null) =>
                    draft.setField("model_preset", !value || value === ORG_DEFAULTS ? "" : value),
                  )}
                >
                  <SelectTrigger id="model-preset" className="w-full bg-background">
                    <SelectValue>
                      {draft.fields.model_preset
                        ? (modelPresets.find((preset) => preset.id === draft.fields.model_preset)
                            ?.name ?? draft.fields.model_preset)
                        : ui("Organization defaults")}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ORG_DEFAULTS}>{ui("Organization defaults")}</SelectItem>
                    {modelPresets.map((preset) => (
                      <SelectItem key={preset.id} value={preset.id}>
                        {preset.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
              <span className="text-xs text-muted-foreground">
                {ui(
                  "Coding, research and small work use this set. A default model above still wins.",
                )}
              </span>
            </ChatChangeMark>

            <HelperModelsSection
              capabilities={draft.capabilities}
              agentPreset={draft.fields.model_preset || undefined}
              readOnly={readOnly}
              onChange={edit(draft.setCapabilities)}
            />

            <ChatChangeMark field="communication" className="flex flex-col gap-1.5">
              <Label className="text-xs font-medium text-muted-foreground" htmlFor="communication">
                {ui("Communication")}
              </Label>
              {readOnly ? (
                <span className="text-[13px] text-muted-foreground">
                  {ui(getCommunicationOption(agent.communication).label)}
                </span>
              ) : (
                <CommunicationSelect
                  id="communication"
                  value={normalizeCommunication(draft.fields.communication)}
                  onValueChange={edit((value: string) => draft.setField("communication", value))}
                  className="bg-background"
                />
              )}
            </ChatChangeMark>
          </>
        )}

        <ChatChangeMark field="tags" className="flex flex-col gap-1.5">
          <Label className="text-xs font-medium text-muted-foreground" htmlFor="tags">
            {ui("Tags")}
          </Label>
          {readOnly ? (
            <div className="flex flex-wrap gap-1">
              {normalizeTags(agent.tags).length === 0 ? (
                <span className="text-[13px] text-muted-foreground">{ui("No tags")}</span>
              ) : (
                normalizeTags(agent.tags).map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))
              )}
            </div>
          ) : (
            <TagInput
              id="tags"
              placeholder={ui("Add a tag…")}
              value={draft.fields.tags}
              onChange={edit((value: string) => draft.setField("tags", value))}
              className="bg-background"
            />
          )}
        </ChatChangeMark>

        <div className="flex flex-col gap-0.5">
          <span className="text-[13px] font-medium">{ui("Updated")}</span>
          <span className="text-[13px] text-muted-foreground">
            {chatChanges?.fields.length
              ? ui("Just now · by Chat")
              : new Date(agent.updated_at).toLocaleString(displayLocale)}
          </span>
        </div>
      </div>

      {external && (
        <p className="border-b p-4 text-xs text-muted-foreground">
          {ui(
            "Harness, capabilities, model, MCP servers, credentials, files, network access, sandbox and token limits are hidden: your endpoint owns them.",
          )}
        </p>
      )}

      <MoreSettingsNav
        rows={moreRows}
        onOpen={onOpenRow}
        renderRow={(row, content) => (
          <ChatChangeMark
            key={row.id}
            field={
              (
                {
                  branding: [
                    "name",
                    "display_name",
                    "description",
                    "intro_markdown",
                    "short_description",
                    "starters",
                  ],
                  files: "initial_files",
                  network: "network_access",
                  sandbox: "sandbox_policy",
                  max_output_tokens: "max_output_tokens",
                  mcp: ["mcpServers", "mcp_servers"],
                  service: "service_virtual_user_id",
                } as Record<string, string | string[]>
              )[row.id] ?? row.id
            }
          >
            {content}
          </ChatChangeMark>
        )}
      />
    </aside>
  );
}
