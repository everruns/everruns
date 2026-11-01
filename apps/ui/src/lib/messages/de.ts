import { messages as sandboxBuilds } from "./de/sandbox-builds";
import { messages as agentCost } from "./de/agent-cost";
import { messages as sharedControls } from "./de/shared-controls";
import { messages as tracing } from "./de/tracing";
import { messages as backgroundLimits } from "./de/background-limits";
import { messages as catalog0 } from "./de/agents-01";
import { messages as catalog1 } from "./de/agents-02";
import { messages as catalog2 } from "./de/agents-03";
import { messages as catalog3 } from "./de/capabilities-01";
import { messages as catalog4 } from "./de/chat-01";
import { messages as catalog5 } from "./de/chat-02";
import { messages as catalog6 } from "./de/common-01";
import { messages as catalog7 } from "./de/common-02";
import { messages as catalog8 } from "./de/common-03";
import { messages as catalog9 } from "./de/common-04";
import { messages as catalog10 } from "./de/common-05";
import { messages as catalog11 } from "./de/common-06";
import { messages as catalog12 } from "./de/common-07";
import { messages as catalog13 } from "./de/common-08";
import { messages as catalog14 } from "./de/harnesses-01";
import { messages as catalog15 } from "./de/operations-01";
import { messages as catalog16 } from "./de/operations-02";
import { messages as catalog17 } from "./de/operations-03";
import { messages as catalog18 } from "./de/settings-01";
import { messages as catalog19 } from "./de/settings-02";
import { messages as catalog20 } from "./de/semantic";
import { messages as externalWorker } from "./de/external-worker";
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
);
