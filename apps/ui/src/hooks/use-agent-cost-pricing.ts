import type { ModelWithProvider } from "@/lib/api/types";
import type { AgentCostPricing } from "@/lib/agent-operating-cost";

// Host override seam: self-hosted model usage is paid to the selected provider.
// A hosted implementation may return effective-dated credit rates for its services.
export function useAgentCostPricing(_model?: ModelWithProvider | null): AgentCostPricing {
  return { status: "ready", modelBilling: "external", rates: [] };
}
