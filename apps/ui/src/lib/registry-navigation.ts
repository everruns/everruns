import { Blocks, BookOpen, Cpu, Plug } from "lucide-react";
import { capabilityIconMap, type IconComponent } from "@/lib/capability-icons";

export interface RegistryNavigationItem {
  name: string;
  href: string;
  icon: IconComponent;
  keywords?: string[];
}

export const registryDomainIcons = {
  models: Cpu,
  mcpServers: capabilityIconMap.mcp,
  skills: BookOpen,
  capabilities: Blocks,
  plugins: Plug,
} satisfies Record<string, IconComponent>;

export const registryNavigationByHref = {
  "/models": {
    name: "Models",
    href: "/models",
    icon: registryDomainIcons.models,
    keywords: ["llm", "openai", "anthropic", "default model"],
  },
  "/mcp-servers": {
    name: "MCP servers",
    href: "/mcp-servers",
    icon: registryDomainIcons.mcpServers,
    keywords: ["mcp", "tool", "integration"],
  },
  "/skills": {
    name: "Skills",
    href: "/skills",
    icon: registryDomainIcons.skills,
    keywords: ["ability", "tool"],
  },
  "/capabilities": {
    name: "Capabilities",
    href: "/capabilities",
    icon: registryDomainIcons.capabilities,
  },
  "/plugins": {
    name: "Plugins",
    href: "/plugins",
    icon: registryDomainIcons.plugins,
    keywords: ["marketplace", "extension", "integration"],
  },
} satisfies Record<string, RegistryNavigationItem>;

export const registryNavigationItems = Object.values(registryNavigationByHref);
