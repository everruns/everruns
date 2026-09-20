#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SLACK_GUIDE="$ROOT/docs/integrations/slack.md"

required_guide_text=(
  'Select **Integrations**.'
  'Select **Add endpoint**.'
  'Select **Save endpoint**.'
  'Select **Publish**'
  'Select **Connect to Slack**'
  '/v1/e/{endpoint_id}/slack/events'
)

for text in "${required_guide_text[@]}"; do
  if ! grep -Fq "$text" "$SLACK_GUIDE"; then
    printf 'Slack guide is missing required current-flow text: %s\n' "$text" >&2
    exit 1
  fi
done

stale_patterns=(
  'App detail page'
  'Go to **Apps**'
  'click **New App**'
  'Click **Create App**'
  'Publish the App'
  '/api/v1/apps'
)

for pattern in "${stale_patterns[@]}"; do
  if grep -RFn --include='*.md' "$pattern" "$ROOT/docs"; then
    printf 'Public docs still instruct readers through the retired App flow: %s\n' "$pattern" >&2
    exit 1
  fi
done

for diagram in \
  "$ROOT/docs/images/integrations/slack-architecture.mmd" \
  "$ROOT/docs/images/integrations/slack-message-flow.mmd"; do
  if ! grep -Fq 'Endpoint' "$diagram"; then
    printf 'Slack diagram does not name Endpoint as the ingress owner: %s\n' "$diagram" >&2
    exit 1
  fi
  if grep -Fq '/v1/apps/' "$diagram"; then
    printf 'Slack diagram still uses an App-scoped ingress route: %s\n' "$diagram" >&2
    exit 1
  fi
done

printf 'Public Slack documentation uses the Agent Integrations endpoint flow.\n'
