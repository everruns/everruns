import { forwardRef, type SVGProps } from "react";
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
 * Custom MCP (Model Context Protocol) icon.
 * Official logo from: https://github.com/modelcontextprotocol/modelcontextprotocol
 */
const McpIcon = forwardRef<SVGSVGElement, SVGProps<SVGSVGElement>>(
  ({ className, ...props }, ref) => (
    <svg
      ref={ref}
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 186 186"
      fill="none"
      className={className}
      {...props}
    >
      <path
        d="M25 97.8528L92.8823 29.9706C102.255 20.598 117.451 20.598 126.823 29.9706V29.9706C136.196 39.3431 136.196 54.5391 126.823 63.9117L75.5581 115.177"
        stroke="currentColor"
        strokeWidth="12"
        strokeLinecap="round"
      />
      <path
        d="M76.2653 114.47L126.823 63.9117C136.196 54.5391 151.392 54.5391 160.765 63.9117L161.118 64.2652C170.491 73.6378 170.491 88.8338 161.118 98.2063L99.7248 159.6C96.6006 162.724 96.6006 167.789 99.7248 170.913L112.331 183.52"
        stroke="currentColor"
        strokeWidth="12"
        strokeLinecap="round"
      />
      <path
        d="M109.853 46.9411L59.6482 97.1457C50.2757 106.518 50.2757 121.714 59.6482 131.087V131.087C69.0208 140.459 84.2168 140.459 93.5894 131.087L143.794 80.8822"
        stroke="currentColor"
        strokeWidth="12"
        strokeLinecap="round"
      />
    </svg>
  )
);
McpIcon.displayName = "McpIcon";

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
  // Custom icons
  mcp: McpIcon as unknown as LucideIcon,
};

/**
 * Get the icon component for a capability.
 * Falls back to CircleOff if the icon is not found.
 */
export function getCapabilityIcon(iconName?: string | null): LucideIcon {
  if (!iconName) return CircleOff;
  return capabilityIconMap[iconName] ?? CircleOff;
}
