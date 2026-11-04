#!/usr/bin/env bash
# Send every committed worst-case Slack manifest to Slack's
# `apps.manifest.validate`, so a limit Slack enforces but does not document
# fails here instead of on a person's Add to Slack.
#
# Needs TEST_SLACK_CONFIG_REFRESH_TOKEN, an app configuration refresh token
# for the Everruns Dev workspace, kept in Doppler everruns-dev/dev. Slack
# configuration access tokens live 12 hours and each rotation issues a new
# refresh token, so the script writes the new one back to Doppler right after
# rotating; DOPPLER_TOKEN must therefore be allowed to write that config.
# Run as: doppler run -- scripts/slack-manifest-validate.sh [fixture.json]
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
fixture="${1:-crates/server/tests/fixtures/slack-worst-case-manifests.json}"
secret_name=TEST_SLACK_CONFIG_REFRESH_TOKEN
api="${SLACK_API_BASE:-https://slack.com/api}"

if [[ -z "${TEST_SLACK_CONFIG_REFRESH_TOKEN:-}" ]]; then
  echo "::error::$secret_name is missing; add it to Doppler everruns-dev/dev" >&2
  exit 1
fi

save_refresh_token() {
  printf '%s' "$1" | doppler secrets set "$secret_name" --no-interactive --silent >/dev/null
}

# Prove write access before spending the refresh token: rotating without
# saving the new one would leave Doppler holding a dead token.
if ! save_refresh_token "$TEST_SLACK_CONFIG_REFRESH_TOKEN"; then
  echo "::error::DOPPLER_TOKEN cannot write $secret_name; it needs write access to everruns-dev/dev" >&2
  exit 1
fi

rotated="$(curl -sS --fail-with-body -X POST "$api/tooling.tokens.rotate" \
  --data-urlencode "refresh_token=$TEST_SLACK_CONFIG_REFRESH_TOKEN")"
if [[ "$(jq -r '.ok' <<<"$rotated")" != "true" ]]; then
  echo "::error::Slack refused the token rotation: $(jq -r '.error' <<<"$rotated")" >&2
  exit 1
fi
access_token="$(jq -r '.token' <<<"$rotated")"
if ! save_refresh_token "$(jq -r '.refresh_token' <<<"$rotated")"; then
  echo "::error::Rotated the Slack token but could not save the new refresh token; generate a new one in Slack and update $secret_name" >&2
  exit 1
fi
unset rotated

failed=0
for case in $(jq -r 'keys[]' "$fixture"); do
  manifest="$(jq -c --arg case "$case" '.[$case]' "$fixture")"
  response="$(curl -sS -X POST "$api/apps.manifest.validate" \
    -H "Authorization: Bearer $access_token" \
    --data-urlencode "manifest=$manifest")"
  if [[ "$(jq -r '.ok' <<<"$response")" == "true" ]]; then
    echo "ok      $case"
  else
    failed=1
    echo "::error::Slack rejected the '$case' manifest: $(jq -c '{error, errors}' <<<"$response")"
  fi
done
exit "$failed"
