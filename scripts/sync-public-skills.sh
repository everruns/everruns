#!/usr/bin/env bash
# Propagate skills/ (the single source of truth, knowledge/project/public-skills.md)
# to a consumer repository and open a pull request there that merges itself
# once that repository's CI is green.
#
#   GH_TOKEN=... scripts/sync-public-skills.sh <plugins|landing>
#
# Each consumer owns its own copy step (plugins: scripts/sync-shared-skills.mjs,
# landing: scripts/sync-agent-skills.mjs); this script only runs it, commits and
# opens the PR, so the list of shared skills lives in one place per consumer.
set -euo pipefail

target="${1:?usage: sync-public-skills.sh <plugins|landing>}"
: "${GH_TOKEN:?GH_TOKEN must be a token that can push to everruns/$target}"
source_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_sha="$(git -C "$source_dir" rev-parse --short HEAD)"

case "$target" in
  plugins) sync=(node scripts/sync-shared-skills.mjs "$source_dir" --bump) ;;
  landing) sync=(node scripts/sync-agent-skills.mjs "$source_dir") ;;
  *) echo "unknown target: $target" >&2; exit 2 ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Hand git the token through askpass so it never lands in a URL, the remote
# config or an error message.
cat > "$work/askpass" <<'ASKPASS'
#!/usr/bin/env sh
case "$1" in
  *Username*) echo "x-access-token" ;;
  *) echo "$GH_TOKEN" ;;
esac
ASKPASS
chmod 700 "$work/askpass"
export GH_TOKEN GIT_ASKPASS="$work/askpass" GIT_TERMINAL_PROMPT=0
git clone --quiet --depth 1 "https://github.com/everruns/${target}.git" "$work/repo"
cd "$work/repo"

"${sync[@]}"
if [ "$target" = plugins ]; then node scripts/build.mjs; fi

if git diff --quiet && [ -z "$(git status --porcelain)" ]; then
  echo "everruns/$target already matches skills/ at $source_sha"
  exit 0
fi

# Commit as the token's owner, so the change is attributed to the person who
# owns the automation rather than to a bot identity.
login="$(gh api user --jq .login)"
id="$(gh api user --jq .id)"
git config user.name "$(gh api user --jq '.name // .login')"
git config user.email "${id}+${login}@users.noreply.github.com"

branch="skills-sync/${source_sha}"
git checkout --quiet -b "$branch"
git add -A
git commit --quiet -m "chore(skills): sync shared skills from everruns ${source_sha}" \
  -m "Copied from everruns/everruns skills/ by scripts/sync-public-skills.sh."
git push --quiet --force origin "$branch"

url="$(gh pr create --repo "everruns/$target" --head "$branch" --base main \
  --title "chore(skills): sync shared skills from everruns ${source_sha}" \
  --body "Automated copy of \`skills/\` from everruns/everruns@${source_sha}. Source of truth: everruns \`knowledge/project/public-skills.md\`. Do not edit the copies here.

Changelog: not user-facing (copy of already-reviewed skills)" 2>/dev/null \
  || gh pr view --repo "everruns/$target" "$branch" --json url --jq .url)"
echo "$url"
# Auto-merge needs "Allow auto-merge" in the repository settings; without it the
# PR stays open and the consumer's drift check keeps flagging it.
gh pr merge --repo "everruns/$target" --auto --squash "$url" \
  || echo "::warning::auto-merge unavailable for everruns/$target; merge $url by hand"
