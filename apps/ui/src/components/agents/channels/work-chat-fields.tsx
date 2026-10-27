"use client";
import { useUiTranslations } from "@/providers/locale-provider";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { ApiFields, type ApiFormState } from "./api-fields";
import { Label } from "@/components/ui/label";

export function WorkChatFields({
  mode,
  onModeChange,
  value,
  onChange,
}: {
  mode: "single_with_threads" | "multi_chat";
  onModeChange: (mode: "single_with_threads" | "multi_chat") => void;
  value: ApiFormState;
  onChange: (value: ApiFormState) => void;
}) {
  const ui = useUiTranslations();
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
      <select
        id="chat-interface"
        className="w-full border bg-background p-2"
        value={mode}
        onChange={(event) => onModeChange(event.target.value as typeof mode)}
      >
        <option value="multi_chat">{ui("Multiple conversations")}</option>
        <option value="single_with_threads" disabled={!features.data?.threads}>
          {ui("Single chat with threads")}
        </option>
      </select>
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
      <ApiFields
        hosted
        value={value}
        onChange={(next) => onChange({ ...next, errors: "public" })}
      />
    </div>
  );
}
