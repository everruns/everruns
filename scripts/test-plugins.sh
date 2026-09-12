#!/usr/bin/env bash
# Validates the portable agent plugins in plugins/ (everruns + resend):
# manifest name/version parity, marketplace registration, and skill frontmatter.
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
check_absent() { # check_absent <desc> <file> <pattern>
  local desc="$1" file="$2" pattern="$3"
  if grep -q "$pattern" "$file" 2>/dev/null; then echo "FAIL - $desc"; fail=1; else echo "ok   - $desc"; fi
}

json_field() { python3 -c "import json,sys; print(json.load(open('$1'))$2)"; }

for plugin in everruns resend; do
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

echo "== marketplaces =="
for m in "$ROOT/.claude-plugin/marketplace.json" "$ROOT/.agents/plugins/marketplace.json" "$ROOT/.cursor-plugin/marketplace.json"; do
  check "$(basename "$(dirname "$m")") marketplace is valid JSON" python3 -c "import json; json.load(open('$m'))"
done
check_contains "everruns in Claude marketplace" "$ROOT/.claude-plugin/marketplace.json" '"everruns"'
check_absent "resend not in Claude marketplace" "$ROOT/.claude-plugin/marketplace.json" '"resend"'
check_contains "everruns in Codex marketplace" "$ROOT/.agents/plugins/marketplace.json" '"everruns"'
check_absent "resend not in Codex marketplace" "$ROOT/.agents/plugins/marketplace.json" '"resend"'
check_contains "everruns in Cursor marketplace" "$ROOT/.cursor-plugin/marketplace.json" '"everruns"'
check_absent "resend not in Cursor marketplace" "$ROOT/.cursor-plugin/marketplace.json" '"resend"'

echo "== MCP endpoints =="
check_contains "everruns host MCP default" "$ROOT/plugins/everruns/.mcp.json" 'https://app.everruns.com/mcp'
check_contains "everruns registry MCP URL" "$ROOT/plugins/everruns/mcp.json" 'https://app.everruns.com/mcp'
check_contains "resend MCP URL" "$ROOT/plugins/resend/mcp.json" 'https://mcp.resend.com/mcp'

echo "== no dev-plugin leftovers =="
if grep -rn "everruns-dev" "$ROOT/plugins" "$ROOT/.claude-plugin" "$ROOT/.agents/plugins" "$ROOT/.cursor-plugin" 2>/dev/null; then
  echo "FAIL - everruns-dev references remain above"; fail=1
else
  echo "ok   - no everruns-dev references in plugin surfaces"
fi

if [ "$fail" -ne 0 ]; then echo "test-plugins: FAILED"; exit 1; fi
echo "test-plugins: all checks passed"
