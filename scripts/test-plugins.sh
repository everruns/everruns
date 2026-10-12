#!/usr/bin/env bash
# Validates the plugins kept in plugins/ (resend): manifest name/version
# parity and skill frontmatter. First-party coding-agent plugins (everruns)
# live in https://github.com/everruns/plugins, which runs its own checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0
check() { # check <desc> <command...>
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then echo "ok   - $desc"; else echo "FAIL - $desc"; fail=1; fi
}
check_contains() { # check_contains <desc> <file> <pattern>
  local desc="$1" file="$2" pattern="$3"
  if grep -q "$pattern" "$file" 2>/dev/null; then echo "ok   - $desc"; else echo "FAIL - $desc"; fail=1; fi
}

json_field() { python3 -c "import json,sys; print(json.load(open('$1'))$2)"; }

for plugin in resend; do
  dir="$ROOT/plugins/$plugin"
  echo "== $plugin =="
  for m in plugin.json .claude-plugin/plugin.json .codex-plugin/plugin.json .cursor-plugin/plugin.json; do
    check "$m is valid JSON" python3 -c "import json; json.load(open('$dir/$m'))"
  done
  name="$(json_field "$dir/plugin.json" "['name']")"
  version="$(json_field "$dir/plugin.json" "['version']")"
  [ "$name" = "$plugin" ] && echo "ok   - root name is $plugin" || { echo "FAIL - root name is $name"; fail=1; }
  for m in .claude-plugin/plugin.json .codex-plugin/plugin.json .cursor-plugin/plugin.json; do
    n="$(json_field "$dir/$m" "['name']")"
    v="$(json_field "$dir/$m" "['version']")"
    [ "$n" = "$plugin" ] && [ "$v" = "$version" ] \
      && echo "ok   - $m name/version ($n@$v)" \
      || { echo "FAIL - $m declares $n@$v, want $plugin@$version"; fail=1; }
  done
  skill="$dir/skills/$plugin/SKILL.md"
  check "$plugin skill exists" test -f "$skill"
  check_contains "$plugin skill frontmatter name" "$skill" "^name: $plugin"
  check_contains "$plugin skill frontmatter description" "$skill" "^description: "
done

echo "== moved to everruns/plugins =="
for gone in plugins/everruns .claude-plugin .agents/plugins .cursor-plugin; do
  if [ -e "$ROOT/$gone" ]; then echo "FAIL - $gone is back; first-party plugins live in everruns/plugins"; fail=1
  else echo "ok   - no $gone"; fi
done
check_contains "default marketplace points at everruns/plugins" \
  "$ROOT/crates/server/src/setup/org_init/mod.rs" 'DEFAULT_MARKETPLACE_REPO: &str = "everruns/plugins"'

echo "== MCP endpoints =="
check_contains "resend MCP URL" "$ROOT/plugins/resend/mcp.json" 'https://mcp.resend.com/mcp'

if [ "$fail" -ne 0 ]; then echo "test-plugins: FAILED"; exit 1; fi
echo "test-plugins: all checks passed"
