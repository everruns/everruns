import {
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  Calculator,
  Globe,
  ListChecks,
  HardDrive,
  CloudSun,
  Cloud,
  Users,
  DollarSign,
  Package,
  type LucideIcon,
} from "lucide-react";

/**
 * Centralized mapping of capability icon names to Lucide React components.
 * Icon names are defined in the backend capability implementations.
 */
export const capabilityIconMap: Record<string, LucideIcon> = {
  // Core capabilities
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
  calculator: Calculator,
  globe: Globe,
  "list-checks": ListChecks,
  "hard-drive": HardDrive,
  "cloud-sun": CloudSun,
  // Additional capability icons
  cloud: Cloud,
  users: Users,
  "dollar-sign": DollarSign,
  package: Package,
};

/**
 * Get the icon component for a capability.
 * Falls back to CircleOff if the icon is not found.
 */
export function getCapabilityIcon(iconName?: string | null): LucideIcon {
  if (!iconName) return CircleOff;
  return capabilityIconMap[iconName] ?? CircleOff;
}
