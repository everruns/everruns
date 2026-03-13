#!/usr/bin/env bash
set -euo pipefail

# Resolve SDK versions per package registry.
# Queries npm, PyPI, and crates.io directly to avoid assuming
# all git tags have corresponding published packages.

fetch_npm_versions() {
  curl -sf "https://registry.npmjs.org/@everruns/sdk" \
    | python3 -c "
import json, sys
data = json.load(sys.stdin)
for v in sorted(data.get('versions', {}).keys(),
                key=lambda x: list(map(int, x.split('.')))):
    print(v)
"
}

fetch_pypi_versions() {
  curl -sf "https://pypi.org/pypi/everruns-sdk/json" \
    | python3 -c "
import json, sys
data = json.load(sys.stdin)
for v in sorted(data.get('releases', {}).keys(),
                key=lambda x: list(map(int, x.split('.')))):
    print(v)
"
}

fetch_crates_versions() {
  curl -sfH "User-Agent: everruns-ci" "https://crates.io/api/v1/crates/everruns-sdk" \
    | python3 -c "
import json, sys
data = json.load(sys.stdin)
versions = [v['num'] for v in data.get('versions', []) if not v.get('yanked')]
for v in sorted(versions, key=lambda x: list(map(int, x.split('.')))):
    print(v)
"
}

# Given an array of sorted versions, compute the cohorts.
# Outputs JSON entries for a single language.
compute_cohorts() {
  local language="$1"
  shift
  local versions=("$@")

  if [ "${#versions[@]}" -eq 0 ]; then
    echo "No versions found for $language" >&2
    return
  fi

  local latest="${versions[${#versions[@]}-1]}"
  IFS='.' read -r latest_major latest_minor latest_patch <<< "$latest"

  local patch_n1="" patch_n2="" last_minor="" last_major=""

  # Find previous patches in same minor
  for ((p=latest_patch-1; p>=0; p--)); do
    local candidate="${latest_major}.${latest_minor}.${p}"
    if printf '%s\n' "${versions[@]}" | grep -Fxq "$candidate"; then
      if [ -z "$patch_n1" ]; then
        patch_n1="$candidate"
      elif [ -z "$patch_n2" ]; then
        patch_n2="$candidate"
        break
      fi
    fi
  done

  # Find highest version from previous minor
  for ((m=latest_minor-1; m>=0; m--)); do
    for v in "${versions[@]}"; do
      IFS='.' read -r vm vn _vp <<< "$v"
      if [ "$vm" -eq "$latest_major" ] && [ "$vn" -eq "$m" ]; then
        last_minor="$v"  # keeps overwriting → highest patch
      fi
    done
    [ -n "$last_minor" ] && break
  done

  # Find highest version from previous major
  for ((M=latest_major-1; M>=0; M--)); do
    for v in "${versions[@]}"; do
      IFS='.' read -r vm _vn _vp <<< "$v"
      if [ "$vm" -eq "$M" ]; then
        last_major="$v"
      fi
    done
    [ -n "$last_major" ] && break
  done

  # Build entries; skip empty cohorts with fallback to latest
  for row in "last_major:$last_major" "last_minor:$last_minor" "patch_n1:$patch_n1" "patch_n2:$patch_n2"; do
    local cohort="${row%%:*}"
    local version="${row#*:}"
    if [ -z "$version" ]; then
      version="$latest"
      cohort="${cohort}_fallback"
    fi
    echo "{\"language\":\"$language\",\"cohort\":\"$cohort\",\"version\":\"$version\"}"
  done
}

echo "Fetching published versions from registries..." >&2

ts_raw="$(fetch_npm_versions 2>/dev/null || true)"
py_raw="$(fetch_pypi_versions 2>/dev/null || true)"
rs_raw="$(fetch_crates_versions 2>/dev/null || true)"

IFS=$'\n' read -r -d '' -a ts_versions <<< "$ts_raw" || true
IFS=$'\n' read -r -d '' -a py_versions <<< "$py_raw" || true
IFS=$'\n' read -r -d '' -a rs_versions <<< "$rs_raw" || true

echo "  npm: ${ts_versions[*]:-none}" >&2
echo "  pypi: ${py_versions[*]:-none}" >&2
echo "  crates.io: ${rs_versions[*]:-none}" >&2

entries=()

add_cohorts() {
  local language="$1"
  shift
  local output
  output="$(compute_cohorts "$language" "$@")"
  while IFS= read -r line; do
    [ -n "$line" ] && entries+=("$line")
  done <<< "$output"
}

add_cohorts typescript "${ts_versions[@]}"
add_cohorts python "${py_versions[@]}"
add_cohorts rust "${rs_versions[@]}"

# Deduplicate: same (language, version) pairs can appear when multiple cohorts
# fall back to the same version. Keep unique combinations.
declare -A seen
unique_entries=()
for entry in "${entries[@]}"; do
  lang=$(echo "$entry" | python3 -c "import json,sys; print(json.load(sys.stdin)['language'])")
  ver=$(echo "$entry" | python3 -c "import json,sys; print(json.load(sys.stdin)['version'])")
  key="${lang}:${ver}"
  if [ -z "${seen[$key]:-}" ]; then
    seen[$key]=1
    unique_entries+=("$entry")
  fi
done

json_array="[$(IFS=,; echo "${unique_entries[*]}")]"

echo "Matrix: $json_array" >&2

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  printf 'matrix=%s\n' "$json_array" >> "$GITHUB_OUTPUT"
else
  echo "$json_array"
fi
