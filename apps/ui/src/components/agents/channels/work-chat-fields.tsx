"use client";
import {
  Select as SharedSelect,
  SelectTrigger as SharedSelectTrigger,
  SelectContent as SharedSelectContent,
  SelectValue as SharedSelectValue,
  SelectItem as SharedSelectItem,
} from "@/components/ui/select";
import { useUiTranslations } from "@/providers/locale-provider";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { ApiFields, type ApiFormState } from "./api-fields";
import { useModels } from "@/hooks";
import { Label } from "@/components/ui/label";

export function WorkChatFields({
  modelIds,
  onModelsChange,
  mode,
  onModeChange,
  value,
  onChange,
}: {
  modelIds: string[];
  onModelsChange: (ids: string[]) => void;
  mode: "single_with_threads" | "multi_chat";
  onModeChange: (mode: "single_with_threads" | "multi_chat") => void;
  value: ApiFormState;
  onChange: (value: ApiFormState) => void;
}) {
  const ui = useUiTranslations();
  const { data: models = [] } = useModels();
  const params = useParams();
  const agentId = String(params.agentId ?? "");
  const features = useQuery({
    queryKey: ["chat-features", agentId],
    enabled: !!agentId,
    queryFn: async () =>
      (
        await api.get<{ threads: boolean; files: boolean }>(
          `/v1/agents/${encodeURIComponent(agentId)}/chat-features`,
        )
      ).data,
  });
  return (
    <div className="space-y-4">
      <Label htmlFor="chat-interface">{ui("Interface")}</Label>
      <SharedSelect
        value={String(mode)}
        onValueChange={(selection) => onModeChange(selection as typeof mode)}
      >
        <SharedSelectTrigger id="chat-interface" className="w-full border bg-background p-2">
          <SharedSelectValue />
        </SharedSelectTrigger>
        <SharedSelectContent>
          <SharedSelectItem value="multi_chat">{ui("Multiple conversations")}</SharedSelectItem>
          <SharedSelectItem value="single_with_threads" disabled={!features.data?.threads}>
            {ui("Single chat with threads")}
          </SharedSelectItem>
        </SharedSelectContent>
      </SharedSelect>
      {!features.data?.threads && (
        <p className="text-sm text-muted-foreground">
          {ui(
            "Single chat with threads requires the Agent’s coordinator capability and thread tools.",
          )}
        </p>
      )}
      <p className="text-sm text-muted-foreground">
        {ui(
          "Consumers sign in separately from the console. Your configured approvals and spending limits apply.",
        )}
      </p>
      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">{ui("Models consumers may select")}</legend>
        <p className="text-sm text-muted-foreground">
          {ui("Leave all unchecked to use the Agent default.")}
        </p>
        <div className="max-h-48 space-y-2 overflow-auto">
          {models
            .filter((model) => model.enabled)
            .map((model) => (
              <label key={model.id} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={modelIds.includes(model.id)}
                  disabled={!modelIds.includes(model.id) && modelIds.length >= 30}
                  onChange={(event) =>
                    onModelsChange(
                      event.target.checked
                        ? [...modelIds, model.id]
                        : modelIds.filter((id) => id !== model.id),
                    )
                  }
                />
                {model.display_name}
              </label>
            ))}
        </div>
      </fieldset>
      <ApiFields
        hosted
        value={value}
        onChange={(next) => onChange({ ...next, errors: "public" })}
      />
    </div>
  );
}
