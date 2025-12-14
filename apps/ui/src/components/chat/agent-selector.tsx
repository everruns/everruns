"use client";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Bot } from "lucide-react";
import type { Agent, AgentVersion } from "@/lib/api/types";

interface AgentSelectorProps {
  agents: Agent[];
  versions: AgentVersion[];
  selectedAgentId: string | null;
  selectedVersion: number | null;
  onAgentChange: (agentId: string) => void;
  onVersionChange: (version: number) => void;
  disabled?: boolean;
}

export function AgentSelector({
  agents,
  versions,
  selectedAgentId,
  selectedVersion,
  onAgentChange,
  onVersionChange,
  disabled,
}: AgentSelectorProps) {
  return (
    <div className="flex items-center gap-3 p-4 border-b">
      <Bot className="h-5 w-5 text-muted-foreground" />
      <div className="flex-1 flex gap-3">
        <Select
          value={selectedAgentId || ""}
          onValueChange={onAgentChange}
          disabled={disabled}
        >
          <SelectTrigger className="w-64">
            <SelectValue placeholder="Select an agent" />
          </SelectTrigger>
          <SelectContent>
            {agents.map((agent) => (
              <SelectItem key={agent.id} value={agent.id}>
                {agent.name}
                <span className="text-muted-foreground ml-2 text-xs">
                  ({agent.default_model_id})
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        {selectedAgentId && versions.length > 0 && (
          <Select
            value={selectedVersion?.toString() || ""}
            onValueChange={(v) => onVersionChange(parseInt(v))}
            disabled={disabled}
          >
            <SelectTrigger className="w-32">
              <SelectValue placeholder="Version" />
            </SelectTrigger>
            <SelectContent>
              {versions.map((version) => (
                <SelectItem key={version.version} value={version.version.toString()}>
                  v{version.version}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>
    </div>
  );
}
