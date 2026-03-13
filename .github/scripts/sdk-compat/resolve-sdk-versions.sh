#!/usr/bin/env bash
set -euo pipefail

# Keep this script portable to stock GitHub runners; avoid non-default tools.
repo_url="https://github.com/everruns/sdk.git"

mapfile -t versions < <(
  git ls-remote --tags --refs "$repo_url" \
    | awk '{print $2}' \
    | sed -E 's#refs/tags/v##' \
    | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' \
    | sort -t. -k1,1n -k2,2n -k3,3n
)

if [ "${#versions[@]}" -eq 0 ]; then
  echo "No SDK versions found in $repo_url" >&2
  exit 1
fi

latest="${versions[${#versions[@]}-1]}"
IFS='.' read -r latest_major latest_minor latest_patch <<< "$latest"

version_exists() {
  local candidate="$1"
  printf '%s\n' "${versions[@]}" | grep -Fxq "$candidate"
}

find_highest_for_constraint() {
  local major="$1"
  local minor="$2"
  local best=""

  for version in "${versions[@]}"; do
    IFS='.' read -r v_major v_minor _v_patch <<< "$version"

    if [ "$major" != "*" ] && [ "$v_major" -ne "$major" ]; then
      continue
    fi

    if [ "$minor" != "*" ] && [ "$v_minor" -ne "$minor" ]; then
      continue
    fi

    best="$version"
  done

  echo "$best"
}

latest_patch_1=""
latest_patch_2=""
last_minor=""
last_major=""

for ((patch=latest_patch-1; patch>=0; patch--)); do
  candidate="${latest_major}.${latest_minor}.${patch}"
  if version_exists "$candidate"; then
    if [ -z "$latest_patch_1" ]; then
      latest_patch_1="$candidate"
    elif [ -z "$latest_patch_2" ]; then
      latest_patch_2="$candidate"
      break
    fi
  fi
done

for ((minor=latest_minor-1; minor>=0; minor--)); do
  candidate="$(find_highest_for_constraint "$latest_major" "$minor")"
  if [ -n "$candidate" ]; then
    last_minor="$candidate"
    break
  fi
done

for ((major=latest_major-1; major>=0; major--)); do
  candidate="$(find_highest_for_constraint "$major" "*")"
  if [ -n "$candidate" ]; then
    last_major="$candidate"
    break
  fi
done

entries=()
for language in typescript python rust; do
  for row in "last_major:$last_major" "last_minor:$last_minor" "patch_n1:$latest_patch_1" "patch_n2:$latest_patch_2"; do
    cohort="${row%%:*}"
    version="${row#*:}"
    if [ -z "$version" ]; then
      if [ "$cohort" = "patch_n2" ]; then
        echo "Required cohort patch_n2 missing in SDK tags" >&2
        exit 1
      fi
      version="$latest"
      cohort="${cohort}_fallback"
    fi
    entries+=("{\"language\":\"$language\",\"cohort\":\"$cohort\",\"version\":\"$version\"}")
  done
done

json_array="[$(IFS=,; echo "${entries[*]}")]"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  printf 'matrix=%s\n' "$json_array" >> "$GITHUB_OUTPUT"
  printf 'latest=%s\n' "$latest" >> "$GITHUB_OUTPUT"
else
  echo "$json_array"
fi
