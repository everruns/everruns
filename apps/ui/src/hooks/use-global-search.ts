/**
 * Aggregates search results from multiple sources for the command palette.
 *
 * Sources:
 * 1. Static navigation pages (always available, no fetch)
 * 2. Agents (client-side filter over cached list)
 * 3. Sessions (client-side filter over first page)
 * 4. Harnesses (client-side filter over cached list)
 * 5. Skills (client-side filter over cached list)
 * 6. ID-based lookup (detects prefixed IDs and provides direct navigation)
 *
 * All entity searches are client-side over already-fetched React Query data.
 * Backend search endpoints will be added incrementally.
 */
"use client";

import { useMemo } from "react";
import {
  LayoutDashboard,
  MessageSquare,
  Shield,
  Boxes,
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

export type SearchResultCategory = "navigation" | "agent" | "session" | "harness" | "skill" | "id";

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
  { title: "Dashboard", href: "/dashboard", icon: LayoutDashboard, keywords: ["home", "overview"] },
  { title: "Sessions", href: "/sessions", icon: MessageSquare, keywords: ["chat", "conversation"] },
  { title: "Chat", href: "/chat", icon: MessageCircle, keywords: ["global chat"] },
  { title: "Agents", href: "/agents", icon: Boxes, keywords: ["bot", "assistant"] },
  { title: "Harnesses", href: "/harnesses", icon: Shield, keywords: ["template", "config"] },
  { title: "Skills", href: "/skills", icon: BookOpen, keywords: ["ability", "tool"] },
  { title: "Capabilities", href: "/capabilities", icon: Puzzle },
  { title: "Apps", href: "/apps", icon: Rocket, keywords: ["deploy", "channel"] },
  { title: "Settings", href: "/settings", icon: Settings, keywords: ["preferences", "config"] },
  {
    title: "Settings > Profile",
    href: "/settings",
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
    href: "/settings/llm-providers",
    icon: Settings,
    keywords: ["openai", "anthropic", "model"],
  },
  {
    title: "Settings > MCP Servers",
    href: "/settings/mcp-servers",
    icon: Settings,
    keywords: ["mcp", "server"],
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
};

function matchesQuery(text: string, query: string): boolean {
  return text.toLowerCase().includes(query);
}

const EMPTY_ARRAY: never[] = [];

export function useGlobalSearch(query: string) {
  const { data: agentsData } = useAgents();
  const { data: sessionsData } = useSessions(undefined, { limit: 100 });
  const { data: harnessesData } = useHarnesses();
  const { data: skillsData } = useSkills();

  const agents = agentsData ?? EMPTY_ARRAY;
  const sessions = sessionsData?.data ?? EMPTY_ARRAY;
  const harnesses = harnessesData ?? EMPTY_ARRAY;
  const skills = skillsData ?? EMPTY_ARRAY;

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

    const results: SearchResult[] = [];
    const MAX_PER_CATEGORY = 5;

    // 1. ID-based lookup
    for (const [prefix, meta] of Object.entries(ID_PREFIX_MAP)) {
      if (q.startsWith(prefix) || q.startsWith(prefix.replace("_", ""))) {
        // Normalize: allow "session3242" or "session_3242"
        const idValue = q.startsWith(prefix) ? q : `${prefix}${q.slice(prefix.length - 1)}`;
        results.push({
          id: `id:${idValue}`,
          category: "id",
          icon: Boxes,
          title: `Go to ${meta.label}`,
          subtitle: idValue,
          href: `${meta.path}/${idValue}`,
        });
      }
    }

    // 2. Navigation pages
    let navCount = 0;
    for (const page of NAVIGATION_PAGES) {
      if (navCount >= MAX_PER_CATEGORY) break;
      const matches =
        matchesQuery(page.title, q) || page.keywords?.some((kw) => matchesQuery(kw, q));
      if (matches) {
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
      if (
        matchesQuery(agent.name, q) ||
        (agent.description && matchesQuery(agent.description, q)) ||
        matchesQuery(agent.id, q)
      ) {
        results.push({
          id: `agent:${agent.id}`,
          category: "agent",
          icon: Boxes,
          title: agent.name,
          subtitle: `Agents > ${agent.name}`,
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
      if (
        matchesQuery(title, q) ||
        matchesQuery(session.id, q) ||
        (session.preview && matchesQuery(session.preview, q))
      ) {
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
      if (
        matchesQuery(harness.name, q) ||
        (harness.description && matchesQuery(harness.description, q)) ||
        matchesQuery(harness.id, q)
      ) {
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
      if (
        matchesQuery(skill.name, q) ||
        (skill.description && matchesQuery(skill.description, q)) ||
        matchesQuery(skill.id, q)
      ) {
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

    return results;
  }, [query, agents, sessions, harnesses, skills]);
}
