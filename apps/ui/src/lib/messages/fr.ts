import { messages as sandboxBuilds } from "./fr/sandbox-builds";
import { messages as agentCost } from "./fr/agent-cost";
import { messages as sharedControls } from "./fr/shared-controls";
import { messages as tracing } from "./fr/tracing";
import { messages as backgroundLimits } from "./fr/background-limits";
import { messages as monitoringTimeline } from "./fr/monitoring-timeline";
import { messages as orgHome } from "./fr/org-home";
import { messages as costs } from "./fr/costs";
import { messages as evals } from "./fr/evals";
import { messages as catalog0 } from "./fr/agents-01";
import { messages as catalog1 } from "./fr/agents-02";
import { messages as catalog2 } from "./fr/agents-03";
import { messages as catalog3 } from "./fr/capabilities-01";
import { messages as catalog4 } from "./fr/chat-01";
import { messages as catalog5 } from "./fr/chat-02";
import { messages as catalog6 } from "./fr/common-01";
import { messages as catalog7 } from "./fr/common-02";
import { messages as catalog8 } from "./fr/common-03";
import { messages as catalog9 } from "./fr/common-04";
import { messages as catalog10 } from "./fr/common-05";
import { messages as catalog11 } from "./fr/common-06";
import { messages as catalog12 } from "./fr/common-07";
import { messages as catalog13 } from "./fr/common-08";
import { messages as catalog14 } from "./fr/harnesses-01";
import { messages as catalog15 } from "./fr/operations-01";
import { messages as catalog16 } from "./fr/operations-02";
import { messages as catalog17 } from "./fr/operations-03";
import { messages as catalog18 } from "./fr/settings-01";
import { messages as catalog19 } from "./fr/settings-02";
import { messages as catalog20 } from "./fr/semantic";
import { messages as externalWorker } from "./fr/external-worker";
export const messages: Readonly<Record<string, string>> = Object.assign(
  Object.create(null),
  sandboxBuilds,
  externalWorker,
  catalog0,
  catalog1,
  catalog2,
  catalog3,
  catalog4,
  catalog5,
  catalog6,
  catalog7,
  catalog8,
  catalog9,
  catalog10,
  catalog11,
  catalog12,
  catalog13,
  catalog14,
  catalog15,
  catalog16,
  catalog17,
  catalog18,
  catalog19,
  catalog20,
  tracing,
  sharedControls,
  agentCost,
  backgroundLimits,
  monitoringTimeline,
  orgHome,
  costs,
  evals,
);
