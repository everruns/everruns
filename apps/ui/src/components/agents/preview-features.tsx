import { Layers } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

const FEATURE_LABELS: Record<string, string> = {
  file_system: "Session filesystem",
  secrets: "Secrets",
  key_value: "Key-value storage",
  sql_database: "SQL databases",
  managed_sandbox: "Managed sandbox",
  leased_resources: "Managed resources",
  subagents: "Subagents",
  agent_handoffs: "Agent handoffs",
  agent_runs: "Agent runs",
  session_tasks: "Session tasks",
  schedules: "Schedules",
  slack_actions: "Slack actions",
  budgeting: "Budgets",
  knowledge: "Knowledge",
  citations: "Citations",
  openui: "Rich UI",
  a2ui: "Agent UI",
};

export function PreviewFeatures({ features }: { features: string[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Layers className="w-5 h-5" />
          Included Features
        </CardTitle>
      </CardHeader>
      <CardContent>
        {features.length === 0 ? (
          <p className="text-sm text-muted-foreground italic">No session features enabled.</p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {features.map((feature) => (
              <Badge key={feature} variant="secondary">
                {FEATURE_LABELS[feature] ??
                  feature.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase())}
              </Badge>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
