---
title: The Eval Builder proposes cases for your agents
description: Ask for eval cases drawn from an agent's instructions and its real sessions, then add the ones you want to a dataset with one click.
date: 2026-11-02
release: "0.51.0"
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: [everruns/everruns#5164]
---

**Propose cases** on an agent's **Evals** tab asks the **Eval Builder** for new
eval cases. It reads the agent's instructions and tools, its recent sessions
(those with findings or failing observer scores first) and the cases its
datasets already have, and suggests up to six: the agent's core job, edge cases
from its instructions, failures seen in real sessions, and adversarial attempts
such as injected instructions. Each comes with a reason, the sessions behind it
and a check.

Nothing is added until you accept a proposal into a dataset. Dismissed ideas
are not offered again.

```bash
everruns evals proposals create --agent-id agent_01933b5a000070008000000000000001
everruns evals proposals accept --proposal-id evalprop_01933b5a000070008000000000000001 \
  --eval-id eval_01933b5a000070008000000000000001
```

See [Evals](/features/evals/).
