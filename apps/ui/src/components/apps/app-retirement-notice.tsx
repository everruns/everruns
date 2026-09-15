import Link from "next/link";
import { Waypoints } from "lucide-react";

import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";

type BoundAgent = {
  id: string;
  name: string;
};

export function AppRetirementNotice({ agents }: { agents: BoundAgent[] }) {
  return (
    <Notice variant="info" icon={<Waypoints className="size-4" />}>
      <NoticeTitle>Apps are moving to agent-owned endpoints</NoticeTitle>
      <NoticeDescription>
        <p>
          Existing Apps and their channels keep working while agent-owned endpoints replace Apps.
        </p>
        {agents.length > 0 && (
          <p>
            {agents.length === 1 ? "Bound agent: " : "Bound agents: "}
            {agents.map((agent, index) => (
              <span key={agent.id}>
                {index > 0 && ", "}
                <Link
                  href={`/agents/${agent.id}`}
                  className="font-medium text-primary hover:underline"
                >
                  {agent.name}
                </Link>
              </span>
            ))}
          </p>
        )}
      </NoticeDescription>
    </Notice>
  );
}
