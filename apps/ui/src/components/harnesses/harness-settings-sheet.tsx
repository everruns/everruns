"use client";

// Side sheet behind the harness page's "More" rows. Large editors (Branding,
// Starter files) need room a narrow column cannot give, so each opens here,
// over the page. Every section edits the page's draft: a change puts the page
// into edit mode, and nothing saves until the header's Save changes.

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";

import { EntityLocalizationsEditor } from "@/components/entity-localizations-editor";
import { Check, Loader2, X } from "lucide-react";
import { useHarnessNameAvailability } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { AgentPromptPane } from "@/components/agents/agent-prompt-pane";
import { StartersEditor } from "@/components/starters-editor";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import { NetworkAccessEditor } from "@/components/network-access-editor";
import type { HarnessDraft } from "@/components/harnesses/use-harness-draft";
import type { Harness } from "@/lib/api/types";
import { pluralize } from "@/lib/formatting";
import { useRetainedSection } from "@/components/workspace/use-retained-section";
import { cn } from "@/lib/utils";

export type HarnessSettingsSection =
  | "localization"
  | "prompt"
  | "branding"
  | "files"
  | "network"
  | "usage";

const EMPTY_PROMPT =
  "This harness contributes no base prompt. The effective prompt comes from the parent harness, agent, session, and capabilities.";

const SECTIONS: Record<
  HarnessSettingsSection,
  { title: string; description: string; kind: "draft" | "info"; wide?: boolean }
> = {
  localization: {
    title: "Localization",
    description: "Localized display names and descriptions.",
    kind: "draft",
  },
  prompt: {
    title: "System prompt",
    description:
      "Optional base instructions. Leave empty to contribute none; the parent harness, agent, session, and capabilities still apply.",
    kind: "draft",
    wide: true,
  },
  branding: {
    title: "Branding",
    description:
      "How the harness is named and how it presents itself when the bound agent leaves a field empty.",
    kind: "draft",
  },
  files: {
    title: "Starter files",
    description: "Files copied into each new session created from this harness.",
    kind: "draft",
    wide: true,
  },
  network: {
    title: "Network access",
    description:
      "Baseline network policy for every agent and session on this harness. Agents and sessions can only narrow it.",
    kind: "draft",
  },
  usage: {
    title: "Usage",
    description: "Sessions and apps using this harness.",
    kind: "info",
  },
};

interface HarnessSettingsSheetProps {
  section: HarnessSettingsSection | null;
  onOpenChange: (open: boolean) => void;
  harness: Harness;
  draft: HarnessDraft;
  editing: boolean;
  readOnly: boolean;
  /** Wraps a draft change so the page enters edit mode. */
  onDraftChange: <T>(apply: (value: T) => void) => (value: T) => void;
  onStartEdit: () => void;
}

export function HarnessSettingsSheet({
  section,
  onOpenChange,
  harness,
  draft,
  editing,
  readOnly,
  onDraftChange,
  onStartEdit,
}: HarnessSettingsSheetProps) {
  const ui = useUiTranslations();
  const active = useRetainedSection(section);
  const meta = active ? SECTIONS[active] : null;

  return (
    <Drawer open={section !== null} onOpenChange={onOpenChange}>
      <DrawerContent
        className={cn("gap-0 overflow-y-auto p-0", meta?.wide ? "sm:max-w-3xl" : "sm:max-w-xl")}
      >
        {active && meta && (
          <>
            <DrawerHeader className="border-b p-5 pr-12">
              <DrawerTitle>{ui(meta.title)}</DrawerTitle>
              <DrawerDescription>{ui(meta.description)}</DrawerDescription>
            </DrawerHeader>
            <div className={cn("flex-1", active === "prompt" ? "" : "p-5")}>
              {active === "prompt" && (
                <AgentPromptPane
                  compact
                  value={draft.fields.system_prompt}
                  editing={editing}
                  onEdit={readOnly ? undefined : onStartEdit}
                  onChange={onDraftChange((value: string) =>
                    draft.setField("system_prompt", value),
                  )}
                  error={draft.errors.system_prompt}
                  emptyMessage={EMPTY_PROMPT}
                  placeholder={ui(
                    "Base instructions for this harness. Leave empty to contribute no prompt.",
                  )}
                />
              )}
              {active === "localization" && (
                <EntityLocalizationsEditor
                  value={draft.localizations}
                  onChange={onDraftChange(draft.setLocalizations)}
                  readOnly={readOnly}
                />
              )}
              {active === "branding" && (
                <BrandingSection
                  harness={harness}
                  draft={draft}
                  readOnly={readOnly}
                  onDraftChange={onDraftChange}
                />
              )}
              {active === "files" && (
                <InitialFilesEditor
                  value={draft.files}
                  onChange={onDraftChange(draft.setFiles)}
                  disabled={readOnly}
                  description={ui("Files copied into each new session created from this harness.")}
                />
              )}
              {active === "network" && (
                <NetworkAccessEditor
                  value={draft.networkAccess}
                  onChange={onDraftChange(draft.setNetworkAccess)}
                  disabled={readOnly}
                  description={ui(
                    "One pattern per line: example.com, *.example.com, or https://example.com/api/.",
                  )}
                />
              )}
              {active === "usage" && <UsageSection harness={harness} />}
            </div>
            <DrawerFooter className="items-center border-t p-4 sm:justify-between">
              <p className="text-xs text-muted-foreground">
                {meta.kind === "draft"
                  ? readOnly
                    ? ui("This harness is read-only.")
                    : ui("Changes are kept with the page edit. Save changes applies them.")
                  : ""}
              </p>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {ui("Done")}
              </Button>
            </DrawerFooter>
          </>
        )}
      </DrawerContent>
    </Drawer>
  );
}

function BrandingSection({
  harness,
  draft,
  readOnly,
  onDraftChange,
}: {
  harness: Harness;
  draft: HarnessDraft;
  readOnly: boolean;
  onDraftChange: HarnessSettingsSheetProps["onDraftChange"];
}) {
  const ui = useUiTranslations();
  const { fields, errors } = draft;
  const nameAvailability = useHarnessNameAvailability(
    draft.nameChanged ? fields.name : "",
    harness.id,
  );
  const field = (key: Parameters<HarnessDraft["setField"]>[0]) =>
    onDraftChange((value: string) => draft.setField(key, value));

  return (
    <div className="flex flex-col gap-5">
      <div className="space-y-2">
        <Label htmlFor="description">{ui("Description")}</Label>
        <Textarea
          id="description"
          placeholder={ui("Describe what this harness does...")}
          value={fields.description}
          onChange={(event) => field("description")(event.target.value)}
          disabled={readOnly}
          rows={2}
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="display_name">{ui("Display name")}</Label>
        <Input
          id="display_name"
          placeholder={fields.name ? undefined : ui("My Harness")}
          value={fields.display_name}
          onChange={(event) => field("display_name")(event.target.value)}
          disabled={readOnly}
        />
        <p className="text-xs text-muted-foreground">
          {ui("Shown in the UI. Defaults to the name when empty.")}
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="name">{ui("Name")}</Label>
        <Input
          id="name"
          placeholder={ui("my-harness")}
          value={fields.name}
          onChange={(event) => field("name")(event.target.value)}
          aria-invalid={!!errors.name}
          disabled={readOnly}
          className="font-mono"
          required
        />
        {errors.name && <p className="text-xs text-destructive">{errors.name}</p>}
        {draft.nameChanged && fields.name.length >= 2 && (
          <div className="flex items-center gap-1.5 text-xs">
            {nameAvailability.isChecking ? (
              <>
                <Loader2 className="size-3 animate-spin text-muted-foreground" />
                <span className="text-muted-foreground">{ui("Checking availability…")}</span>
              </>
            ) : nameAvailability.available === true ? (
              <>
                <Check className="size-3 text-success" />
                <span className="text-success">{ui("Name is available")}</span>
              </>
            ) : nameAvailability.available === false ? (
              <>
                <X className="size-3 text-destructive" />
                <span className="text-destructive">{ui("Name is already taken or invalid")}</span>
              </>
            ) : null}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          {ui(
            "Unique identifier used in URLs and the API. Lowercase letters, numbers, and hyphens.",
          )}
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="intro_markdown">{ui("Intro (Markdown)")}</Label>
        <Textarea
          id="intro_markdown"
          value={fields.intro_markdown}
          onChange={(event) => field("intro_markdown")(event.target.value)}
          placeholder={"I can triage incidents, dig through logs, and draft the update."}
          disabled={readOnly}
          rows={4}
        />
        <p className="text-xs text-muted-foreground">
          {ui(
            "Shown as an intro box on a fresh thread when the bound agent leaves the field empty. Images are allowed. Hidden once the user types.",
          )}
        </p>
        {errors.intro_markdown && (
          <p className="text-xs text-destructive">{errors.intro_markdown}</p>
        )}
      </div>

      <div className="space-y-2">
        <Label htmlFor="short_description">{ui("Short description")}</Label>
        <Input
          id="short_description"
          value={fields.short_description}
          onChange={(event) => field("short_description")(event.target.value)}
          placeholder={ui("Triage incidents, dig through logs, draft the update.")}
          disabled={readOnly}
          maxLength={2048}
        />
        <p className="text-xs text-muted-foreground">
          {ui(
            "One line in simplified Markdown, shown below the chat title once the intro hides. The agent's value wins.",
          )}
        </p>
        {errors.short_description && (
          <p className="text-xs text-destructive">{errors.short_description}</p>
        )}
      </div>

      {!readOnly && (
        <StartersEditor
          value={draft.starters}
          onChange={onDraftChange(draft.setStarters)}
          error={errors.starters}
        />
      )}
    </div>
  );
}

function UsageSection({ harness }: { harness: Harness }) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  const sessions = harness.session_count ?? 0;
  const apps = harness.app_count ?? 0;

  return (
    <p className="text-sm text-muted-foreground">
      {sessions} {pluralize(sessions, "session", undefined, displayLocale)} {ui("and")} {apps}{" "}
      {pluralize(apps, "app", undefined, displayLocale)}{" "}
      {ui("use this harness. The Stats tab breaks usage down over time.")}
    </p>
  );
}
