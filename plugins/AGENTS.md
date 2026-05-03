## Plugin changes

When changing a shipped plugin, bump the plugin patch version unless the change
is purely internal test code. Keep all plugin manifests and marketplace entries
that carry the version in sync, then run the plugin metadata validation.

For `plugins/everruns-dev`, update:

- `plugins/everruns-dev/.codex-plugin/plugin.json`
- `plugins/everruns-dev/.claude-plugin/plugin.json`
- `.claude-plugin/marketplace.json`

Validate with:

```bash
bash scripts/test-everruns-dev-plugin.sh
```
