import {
  CapabilitiesIcon,
  ModelsIcon,
  PluginsIcon,
  SkillsIcon,
} from "@/components/icons/facet-icons";
import { capabilityIconMap, type IconComponent } from "@/lib/capability-icons";

export interface RegistryNavigationItem {
  name: string;
  href: string;
  icon: IconComponent;
  keywords?: string[];
}

export const registryDomainIcons = {
  models: ModelsIcon,
  mcpServers: capabilityIconMap.mcp,
  skills: SkillsIcon,
  capabilities: CapabilitiesIcon,
  plugins: PluginsIcon,
} satisfies Record<string, IconComponent>;

// The org MCP catalog is not a register: it is an admin registry under
// Settings > Organization (knowledge/integrations/user-mcp-servers.md, step 8).
export const registryNavigationByHref = {
  "/models": {
    name: "Models",
    href: "/models",
    icon: registryDomainIcons.models,
    keywords: [
      "llm",
      "openai",
      "anthropic",
      "default model",
      "providers",
      "credentials",
      "api key",
    ],
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
