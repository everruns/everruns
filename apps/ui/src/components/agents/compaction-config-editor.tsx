"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import { COMPACTION_STRATEGIES } from "@/lib/compaction";

export function CompactionConfigEditor({
  config,
  onChange,
  disabled,
}: {
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}) {
  const ui = useUiTranslations();
  const strategy = typeof config.strategy === "string" ? config.strategy : "auto";
  const working =
    typeof config.context_budget_tokens === "number" ? config.context_budget_tokens : 64000;
  const summary =
    config.summarization && typeof config.summarization === "object"
      ? (config.summarization as Record<string, unknown>)
      : {};
  const set = (key: string, value: unknown) => onChange({ ...config, [key]: value });
  const setSummary = (key: string, value: string) =>
    set("summarization", { ...summary, [key]: value || null });
  return (
    <div className="space-y-4 py-3">
      <p className="text-xs text-muted-foreground">
        {ui(
          "Keeps recent work in context and older history retrievable. Summary calls may add cost.",
        )}
      </p>
      <div className="space-y-1.5">
        <Label className="flex-col items-start gap-1.5">
          {ui("Working history budget")}
          <Input
            aria-label={ui("Working history budget")}
            type="number"
            min={4096}
            step={1024}
            value={working}
            disabled={disabled}
            aria-invalid={working < 4096}
            onChange={(event) => set("context_budget_tokens", Number(event.target.value))}
          />
        </Label>
        <p className="text-xs text-muted-foreground">
          {ui("Estimated tokens; separate from the model's maximum window.")}
        </p>
        {working < 4096 && (
          <p role="alert" className="text-xs text-destructive">
            {ui("Use at least 4,096 tokens.")}
          </p>
        )}
      </div>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor="compaction-proactive">{ui("Proactive compaction")}</Label>
        <Switch
          id="compaction-proactive"
          checked={config.proactive !== false}
          disabled={disabled}
          onCheckedChange={(value) => set("proactive", value)}
        />
      </div>
      <details className="border-t pt-3">
        <summary className="cursor-pointer text-sm font-medium">{ui("Advanced")}</summary>
        <div className="space-y-4 pt-4">
          <div className="space-y-1.5">
            <Label>{ui("Strategy")}</Label>
            <Select
              value={strategy}
              onValueChange={(value) => value && set("strategy", value)}
              disabled={disabled}
            >
              <SelectTrigger aria-label={ui("Strategy")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.entries(COMPACTION_STRATEGIES)
                  .filter(([value]) =>
                    ["auto", "native", "observation_masking", "summarization"].includes(value),
                  )
                  .map(([value, label]) => (
                    <SelectItem key={value} value={value}>
                      {ui(label)}
                    </SelectItem>
                  ))}
              </SelectContent>
            </Select>
          </div>
          {(strategy === "auto" || strategy === "native") && (
            <Label className="flex-col items-start gap-1.5">
              {ui("Native trigger (% of model window)")}
              <Input
                aria-label={ui("Native trigger (% of model window)")}
                type="number"
                min={10}
                max={100}
                value={Math.round(Number(config.budget_percent ?? 0.85) * 100)}
                disabled={disabled}
                onChange={(event) => set("budget_percent", Number(event.target.value) / 100)}
              />
            </Label>
          )}
          {(strategy === "auto" || strategy === "summarization") && (
            <>
              <Label className="flex-col items-start gap-1.5">
                {ui("Summary model")}
                <Input
                  aria-label={ui("Summary model")}
                  value={typeof summary.model === "string" ? summary.model : ""}
                  placeholder={ui("Same as conversation model")}
                  disabled={disabled}
                  onChange={(event) => setSummary("model", event.target.value)}
                />
              </Label>
              <p className="text-xs text-muted-foreground">
                {ui("Use a model name supported by the current provider.")}
              </p>
              <Label className="flex-col items-start gap-1.5">
                {ui("Summary instructions")}
                <Textarea
                  aria-label={ui("Summary instructions")}
                  value={typeof summary.instructions === "string" ? summary.instructions : ""}
                  disabled={disabled}
                  onChange={(event) => setSummary("instructions", event.target.value)}
                />
              </Label>
            </>
          )}
        </div>
      </details>
    </div>
  );
}
