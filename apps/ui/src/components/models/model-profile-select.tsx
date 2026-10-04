"use client";
import { useId } from "react";
import { useModelProfiles } from "@/hooks/use-providers";
import type { ModelProfileResponse, ModelService } from "@/lib/api/types";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
export function ModelProfileSelect({
  providerId,
  service,
  value,
  onChange,
}: {
  providerId: string;
  service: ModelService;
  value?: string;
  onChange: (profile?: ModelProfileResponse) => void;
}) {
  const id = useId();
  const { data: profiles = [] } = useModelProfiles(providerId, service);
  const selected = profiles.find((p) => p.key === value);
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>Model profile</Label>
      <Select
        value={value || "custom"}
        onValueChange={(key) => onChange(profiles.find((p) => p.key === key))}
      >
        <SelectTrigger id={id} className="w-full">
          <SelectValue>{selected?.profile.name ?? value ?? "Custom model"}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="custom">Custom model</SelectItem>
          {profiles.map((p) => (
            <SelectItem key={p.key} value={p.key}>
              {p.profile.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {selected && (
        <p className="text-xs text-muted-foreground">
          {selected.key}
          {selected.profile.decisions &&
            ` · ${selected.profile.decisions.calibrated ? "Calibrated" : "Uncalibrated"} · ${selected.profile.decisions.primitives.join(", ")} · ${selected.profile.decisions.max_choice_options} choices · ${selected.profile.decisions.max_score_levels} score levels`}
        </p>
      )}
    </div>
  );
}
