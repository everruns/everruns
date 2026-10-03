# Everruns DeepSec setup

The scaffold and coverage setup are complete. The checked-in `INFO.md`
describes the current threat model, and `../../generated-matchers.ts` contains
the scoped matchers created by coverage analysis.

From `.deepsec/`, inspect checkpoints or resume setup with:

```bash
pnpm deepsec setup --project-id everruns --status --output json
pnpm deepsec setup --project-id everruns --agent codex --model-auth local --yes
```

See `../../README.md` and `../../node_modules/deepsec/SKILL.md` for the current
workflow.
