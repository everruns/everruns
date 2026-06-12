"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Plus, X } from "lucide-react";

export interface KeyValuePair {
  key: string;
  value: string;
}

export function KeyValueEditor({
  label,
  pairs,
  onChange,
  keyPlaceholder = "Name",
  valuePlaceholder = "Value",
  addLabel = "Add",
}: {
  label: string;
  pairs: KeyValuePair[];
  onChange: (pairs: KeyValuePair[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  addLabel?: string;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label>{label}</Label>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 text-xs"
          onClick={() => onChange([...pairs, { key: "", value: "" }])}
        >
          <Plus className="h-3 w-3 mr-1" />
          {addLabel}
        </Button>
      </div>
      {pairs.length > 0 && (
        <div className="space-y-2">
          {pairs.map((pair, index) => (
            <div key={index} className="flex items-center gap-2">
              <Input
                placeholder={keyPlaceholder}
                value={pair.key}
                onChange={(e) =>
                  onChange(pairs.map((p, i) => (i === index ? { ...p, key: e.target.value } : p)))
                }
                className="flex-1"
              />
              <Input
                placeholder={valuePlaceholder}
                value={pair.value}
                onChange={(e) =>
                  onChange(pairs.map((p, i) => (i === index ? { ...p, value: e.target.value } : p)))
                }
                className="flex-1"
              />
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-9 w-9 p-0 shrink-0"
                onClick={() => onChange(pairs.filter((_, i) => i !== index))}
              >
                <X className="h-4 w-4" />
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
