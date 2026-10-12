import { forwardRef, type SVGProps } from "react";
import { facetIconData, facetStrokeWidth, type FacetIconName } from "./facet-icon-data";

type FacetIconProps = SVGProps<SVGSVGElement> & { size?: number | string };

function createFacetIcon(name: FacetIconName) {
  const data: { solid?: boolean; paths: readonly string[] } = facetIconData[name];
  const Icon = forwardRef<SVGSVGElement, FacetIconProps>(
    ({ size = 24, children, ...props }, ref) => (
      <svg
        ref={ref}
        xmlns="http://www.w3.org/2000/svg"
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill={data.solid ? "currentColor" : "none"}
        stroke={data.solid ? "none" : "currentColor"}
        strokeWidth={data.solid ? undefined : facetStrokeWidth}
        strokeLinecap="square"
        strokeLinejoin="miter"
        aria-hidden="true"
        focusable="false"
        data-facet-icon={name}
        {...props}
      >
        {data.paths.map((d) => (
          <path key={d} d={d} />
        ))}
        {children}
      </svg>
    ),
  );
  Icon.displayName = `Facet(${name})`;
  return Icon;
}

export const AgentIcon = createFacetIcon("agent");
export const ChatIcon = createFacetIcon("chat");
export const PlaygroundIcon = createFacetIcon("playground");
export const HarnessDomainIcon = createFacetIcon("harness");
export const VirtualUserIcon = createFacetIcon("virtualUser");
export const SessionIcon = createFacetIcon("session");
export const ExposureIcon = createFacetIcon("exposure");
export const ModelsIcon = createFacetIcon("models");
export const SkillsIcon = createFacetIcon("skills");
export const CapabilitiesIcon = createFacetIcon("capabilities");
export const PluginsIcon = createFacetIcon("plugins");
export const KnowledgeIcon = createFacetIcon("knowledge");
export const MemoryIcon = createFacetIcon("memory");
export const EvalsIcon = createFacetIcon("evals");
export const ObserverIcon = createFacetIcon("observer");
export const ReportIcon = createFacetIcon("report");
export const SandboxIcon = createFacetIcon("sandbox");
export const SandboxTemplateIcon = createFacetIcon("sandboxTemplate");
export const ProviderAccountIcon = createFacetIcon("providerAccount");
export const SettingsIcon = createFacetIcon("settings");
export const DurableIcon = createFacetIcon("durable");
export const WorkerIcon = createFacetIcon("worker");
export const WorkflowIcon = createFacetIcon("workflow");
export const QueueIcon = createFacetIcon("queue");
export const ScheduleIcon = createFacetIcon("schedule");
export const CircuitBreakerIcon = createFacetIcon("circuitBreaker");
export const OrganizationIcon = createFacetIcon("organization");
export const ProviderIcon = createFacetIcon("provider");
export const TeamIcon = createFacetIcon("team");
export const HealthIcon = createFacetIcon("health");
export const FeaturesIcon = createFacetIcon("features");
export const PaymentsIcon = createFacetIcon("payments");
export const AccountIcon = createFacetIcon("account");
export const AgentExperienceIcon = createFacetIcon("agentExperience");
export const TokenIcon = createFacetIcon("token");
