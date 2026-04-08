/**
 * Aggregates search results from multiple sources for the command palette.
 *
 * Sources:
 * 1. Static navigation pages (always available, no fetch)
 * 2. Agents (client-side filter over cached list)
 * 3. Sessions (client-side filter over first page)
 * 4. Harnesses (client-side filter over cached list)
 * 5. Skills (client-side filter over cached list)
 * 6. MCP Servers (client-side filter over cached list)
 * 7. Capabilities (client-side filter over cached list)
 * 8. ID-based lookup (detects prefixed IDs and provides direct navigation)
 * 9. Evals (client-side filter over cached list)
 * 10. Apps (client-side filter over cached list)
 * 11. Agent Identities (client-side filter over cached list)
 *
 * All entity searches are client-side over already-fetched React Query data.
 * Backend search endpoints are available for server-side filtering when needed.
 */
"use client";

import { useMemo } from "react";
import {
  LayoutDashboard,
  MessageSquare,
  Shield,
  Boxes,
  UserRound,
  BookOpen,
  Puzzle,
  Rocket,
  Settings,
  Cog,
  Server,
  Workflow,
  ListTodo,
  Calendar,
  MessageCircle,
  FlaskConical,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useAgents } from "@/hooks/use-agents";
import { useSessions } from "@/hooks/use-sessions";
import { useHarnesses } from "@/hooks/use-harnesses";
import { useSkills } from "@/hooks/use-skills";
import { useMcpServers } from "@/hooks/use-mcp-servers";
import { useCapabilities } from "@/hooks/use-capabilities";
import { useEvals } from "@/hooks/use-evals";
import { useApps } from "@/hooks/use-apps";
import { useAgentIdentities } from "@/hooks/use-agent-identities";
import { getDisplayName } from "@/lib/entity-lifecycle";

export type SearchResultCategory =
  | "navigation"
  | "agent"
  | "agent_identity"
  | "session"
  | "harness"
  | "skill"
  | "mcp_server"
  | "capability"
  | "app"
  | "eval"
  | "id";

export interface SearchResult {
  id: string;
  category: SearchResultCategory;
  icon: LucideIcon;
  title: string;
  /** Breadcrumb-style subtitle, e.g. "Agents > Daytona Coder" */
  subtitle?: string;
  href: string;
}

interface NavigationPage {
  title: string;
  href: string;
  icon: LucideIcon;
  keywords?: string[];
}

const NAVIGATION_PAGES: NavigationPage[] = [
  {
    title: "Dashboard",
    href: "/dashboard",
    icon: LayoutDashboard,
    keywords: ["home", "overview"],
  },
  {
    title: "Sessions",
    href: "/sessions",
    icon: MessageSquare,
    keywords: ["chat", "conversation"],
  },
  {
    title: "Chat",
    href: "/chat",
    icon: MessageCircle,
    keywords: ["global chat"],
  },
  {
    title: "Agents",
    href: "/agents",
    icon: Boxes,
    keywords: ["bot", "assistant"],
  },
  {
    title: "Agent Identities",
    href: "/agent-identities",
    icon: UserRound,
    keywords: ["persona", "principal", "identity"],
  },
  {
    title: "Harnesses",
    href: "/harnesses",
    icon: Shield,
    keywords: ["template", "config"],
  },
  {
    title: "Skills",
    href: "/skills",
    icon: BookOpen,
    keywords: ["ability", "tool"],
  },
  { title: "Capabilities", href: "/capabilities", icon: Puzzle },
  {
    title: "Apps",
    href: "/apps",
    icon: Rocket,
    keywords: ["deploy", "channel"],
  },
  {
    title: "Evals",
    href: "/evals",
    icon: FlaskConical,
    keywords: ["evaluation", "test", "benchmark", "score"],
  },
  {
    title: "MCP Servers",
    href: "/mcp-servers",
    icon: Server,
    keywords: ["mcp", "tool", "integration"],
  },
  {
    title: "Settings",
    href: "/settings",
    icon: Settings,
    keywords: ["preferences", "config"],
  },
  {
    title: "Settings > Profile",
    href: "/settings/profile",
    icon: Settings,
    keywords: ["account", "profile"],
  },
  {
    title: "Settings > API Keys",
    href: "/settings/api-keys",
    icon: Settings,
    keywords: ["token", "key"],
  },
  {
    title: "Settings > Connections",
    href: "/settings/connections",
    icon: Settings,
    keywords: ["github", "gitlab"],
  },
  {
    title: "Settings > LLM Providers",
    href: "/settings/providers",
    icon: Settings,
    keywords: ["openai", "anthropic", "model"],
  },
  {
    title: "Settings > Organisation",
    href: "/settings/organisation",
    icon: Settings,
    keywords: ["org", "team"],
  },
  {
    title: "Settings > Members",
    href: "/settings/members",
    icon: Settings,
    keywords: ["team", "invite"],
  },
  {
    title: "Durable Execution",
    href: "/durable",
    icon: Cog,
    keywords: ["workflow", "worker", "queue"],
  },
  { title: "Durable > Workers", href: "/durable/workers", icon: Server },
  { title: "Durable > Workflows", href: "/durable/workflows", icon: Workflow },
  { title: "Durable > Queues", href: "/durable/queues", icon: ListTodo },
  { title: "Durable > Schedules", href: "/durable/schedules", icon: Calendar },
  { title: "Dev Tools", href: "/dev", icon: FlaskConical },
];

/** Known ID prefixes and where they resolve. */
const ID_PREFIX_MAP: Record<
  string,
  { category: SearchResultCategory; label: string; path: string }
> = {
  agent_: { category: "agent", label: "Agent", path: "/agents" },
  session_: { category: "session", label: "Session", path: "/sessions" },
  harness_: { category: "harness", label: "Harness", path: "/harnesses" },
  skill_: { category: "skill", label: "Skill", path: "/skills" },
  mcp_: { category: "mcp_server", label: "MCP Server", path: "/mcp-servers" },
  eval_: { category: "eval", label: "Eval", path: "/evals" },
  app_: { category: "app", label: "App", path: "/apps" },
  identity_: {
    category: "agent_identity",
    label: "Agent Identity",
    path: "/agent-identities",
  },
};

/**
 * Tokenized multi-word search. Every word in the query must appear
 * somewhere in the combined searchable text. This means "Daytona Agent"
 * matches an agent named "Daytona Coder" because "daytona" hits the name
 * and "agent" hits the category context passed via `extraContext`.
 */
function matchesTokens(tokens: string[], ...texts: (string | undefined | null)[]): boolean {
  const combined = texts
    .filter(Boolean)
    .map((t) => t!.toLowerCase())
    .join(" ");
  return tokens.every((token) => combined.includes(token));
}

const EMPTY_ARRAY: never[] = [];

export function useGlobalSearch(query: string) {
  const { data: agentsData } = useAgents();
  const { data: sessionsData } = useSessions(undefined, { limit: 100 });
  const { data: harnessesData } = useHarnesses();
  const { data: skillsData } = useSkills();
  const { data: mcpServersData } = useMcpServers();
  const { data: capabilitiesData } = useCapabilities();
  const { data: evalsData } = useEvals();
  const { data: appsData } = useApps();
  const { data: agentIdentitiesData } = useAgentIdentities();

  const agents = agentsData ?? EMPTY_ARRAY;
  const sessions = sessionsData?.data ?? EMPTY_ARRAY;
  const harnesses = harnessesData ?? EMPTY_ARRAY;
  const skills = skillsData ?? EMPTY_ARRAY;
  const mcpServers = mcpServersData ?? EMPTY_ARRAY;
  const capabilities = capabilitiesData ?? EMPTY_ARRAY;
  const evals = evalsData ?? EMPTY_ARRAY;
  const apps = appsData ?? EMPTY_ARRAY;
  const agentIdentities = agentIdentitiesData ?? EMPTY_ARRAY;

  return useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      // Show top navigation pages when empty
      return NAVIGATION_PAGES.slice(0, 6).map(
        (page): SearchResult => ({
          id: `nav:${page.href}`,
          category: "navigation",
          icon: page.icon,
          title: page.title,
          href: page.href,
        }),
      );
    }

    const tokens = q.split(/\s+/).filter(Boolean);
    const results: SearchResult[] = [];
    const MAX_PER_CATEGORY = 5;

    // 1. ID-based lookup — resolve entity name from cached data when possible
    for (const [prefix, meta] of Object.entries(ID_PREFIX_MAP)) {
      if (q.startsWith(prefix) || q.startsWith(prefix.replace("_", ""))) {
        // Normalize: allow "session3242" or "session_3242"
        const idValue = q.startsWith(prefix) ? q : `${prefix}${q.slice(prefix.length - 1)}`;

        // Try to resolve a friendly name from cached data
        let resolvedName: string | undefined;
        if (prefix === "agent_") {
          const a = agents.find((a) => a.id === idValue);
          resolvedName = a ? getDisplayName(a) : undefined;
        } else if (prefix === "session_") {
          const s = sessions.find((s) => s.id === idValue);
          resolvedName = s?.title ?? s?.preview ?? undefined;
        } else if (prefix === "harness_") {
          resolvedName = harnesses.find((h) => h.id === idValue)?.name;
        } else if (prefix === "skill_") {
          resolvedName = skills.find((s) => s.id === idValue)?.name;
        } else if (prefix === "mcp_") {
          resolvedName = mcpServers.find((m) => m.id === idValue)?.name;
        } else if (prefix === "eval_") {
          resolvedName = evals.find((e) => e.id === idValue)?.name;
        } else if (prefix === "app_") {
          resolvedName = apps.find((a) => a.id === idValue)?.name;
        } else if (prefix === "identity_") {
          resolvedName = agentIdentities.find((ai) => ai.id === idValue)?.name;
        }

        results.push({
          id: `id:${idValue}`,
          category: "id",
          icon: Boxes,
          title: resolvedName ? `${meta.label}: ${resolvedName}` : `Go to ${meta.label}`,
          subtitle: idValue,
          href: `${meta.path}/${idValue}`,
        });
      }
    }

    // 2. Navigation pages
    let navCount = 0;
    for (const page of NAVIGATION_PAGES) {
      if (navCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, page.title, ...(page.keywords ?? []))) {
        results.push({
          id: `nav:${page.href}`,
          category: "navigation",
          icon: page.icon,
          title: page.title,
          href: page.href,
        });
        navCount++;
      }
    }

    // 3. Agents
    let agentCount = 0;
    for (const agent of agents) {
      if (agentCount >= MAX_PER_CATEGORY) break;
      const agentDisplayName = getDisplayName(agent);
      if (
        matchesTokens(tokens, agent.name, agentDisplayName, agent.description, agent.id, "agent")
      ) {
        results.push({
          id: `agent:${agent.id}`,
          category: "agent",
          icon: Boxes,
          title: agentDisplayName,
          subtitle: `Agents > ${agentDisplayName}`,
          href: `/agents/${agent.id}`,
        });
        agentCount++;
      }
    }

    // 4. Sessions
    let sessionCount = 0;
    for (const session of sessions) {
      if (sessionCount >= MAX_PER_CATEGORY) break;
      const title = session.title || session.preview || session.id;
      if (matchesTokens(tokens, title, session.id, session.preview, "session")) {
        results.push({
          id: `session:${session.id}`,
          category: "session",
          icon: MessageSquare,
          title: title,
          subtitle: `Sessions > ${title.length > 40 ? title.slice(0, 40) + "..." : title}`,
          href: `/sessions/${session.id}`,
        });
        sessionCount++;
      }
    }

    // 5. Harnesses
    let harnessCount = 0;
    for (const harness of harnesses) {
      if (harnessCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, harness.name, harness.description, harness.id, "harness")) {
        results.push({
          id: `harness:${harness.id}`,
          category: "harness",
          icon: Shield,
          title: harness.name,
          subtitle: `Harnesses > ${harness.name}`,
          href: `/harnesses/${harness.id}`,
        });
        harnessCount++;
      }
    }

    // 6. Skills
    let skillCount = 0;
    for (const skill of skills) {
      if (skillCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, skill.name, skill.description, skill.id, "skill")) {
        results.push({
          id: `skill:${skill.id}`,
          category: "skill",
          icon: BookOpen,
          title: skill.name,
          subtitle: `Skills > ${skill.name}`,
          href: `/skills/${skill.id}`,
        });
        skillCount++;
      }
    }

    // 7. MCP Servers
    let mcpCount = 0;
    for (const server of mcpServers) {
      if (mcpCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, server.name, server.description, server.id, "mcp server")) {
        results.push({
          id: `mcp:${server.id}`,
          category: "mcp_server",
          icon: Server,
          title: server.name,
          subtitle: `MCP Servers > ${server.name}`,
          href: `/mcp-servers`,
        });
        mcpCount++;
      }
    }

    // 8. Capabilities
    let capabilityCount = 0;
    for (const capability of capabilities) {
      if (capabilityCount >= MAX_PER_CATEGORY) break;
      if (
        matchesTokens(
          tokens,
          capability.name,
          capability.description,
          capability.id,
          capability.category,
          "capability",
        )
      ) {
        results.push({
          id: `capability:${capability.id}`,
          category: "capability",
          icon: Puzzle,
          title: capability.name,
          subtitle: `Capabilities > ${capability.name}`,
          href: `/capabilities/${capability.id}`,
        });
        capabilityCount++;
      }
    }

    // 9. Evals
    let evalCount = 0;
    for (const ev of evals) {
      if (evalCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, ev.name, ev.description, ev.id, "eval")) {
        results.push({
          id: `eval:${ev.id}`,
          category: "eval",
          icon: FlaskConical,
          title: ev.name,
          subtitle: `Evals > ${ev.name}`,
          href: `/evals/${ev.id}`,
        });
        evalCount++;
      }
    }

    // 10. Apps
    let appCount = 0;
    for (const app of apps) {
      if (appCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, app.name, app.description, app.id, "app")) {
        results.push({
          id: `app:${app.id}`,
          category: "app",
          icon: Rocket,
          title: app.name,
          subtitle: `Apps > ${app.name}`,
          href: `/apps/${app.id}`,
        });
        appCount++;
      }
    }

    // 11. Agent Identities
    let identityCount = 0;
    for (const identity of agentIdentities) {
      if (identityCount >= MAX_PER_CATEGORY) break;
      if (
        matchesTokens(tokens, identity.name, identity.description, identity.id, "agent identity")
      ) {
        results.push({
          id: `identity:${identity.id}`,
          category: "agent_identity",
          icon: UserRound,
          title: identity.name,
          subtitle: `Agent Identities > ${identity.name}`,
          href: `/agent-identities/${identity.id}`,
        });
        identityCount++;
      }
    }

    return results;
  }, [
    query,
    agents,
    sessions,
    harnesses,
    skills,
    mcpServers,
    capabilities,
    evals,
    apps,
    agentIdentities,
  ]);
}
