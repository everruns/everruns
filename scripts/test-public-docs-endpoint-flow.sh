#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SLACK_GUIDE="$ROOT/docs/integrations/slack.md"

required_guide_text=(
  'Select **Integrations**.'
  'Select **Add channel**.'
  'Select **Save channel**.'
  'Select **Publish**'
  'Select **Connect to Slack**'
  '/v1/e/{channel_id}/slack/events'
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
  'Add endpoint'
  'Save endpoint'
)

for pattern in "${stale_patterns[@]}"; do
  if grep -RFn --include='*.md' "$pattern" "$ROOT/docs"; then
    printf 'Public docs still instruct readers through a retired App or Endpoint flow: %s\n' "$pattern" >&2
    exit 1
  fi
done

for diagram in \
  "$ROOT/docs/images/integrations/slack-architecture.mmd" \
  "$ROOT/docs/images/integrations/slack-message-flow.mmd"; do
  if ! grep -Fq 'Channel' "$diagram"; then
    printf 'Slack diagram does not name Channel as the ingress owner: %s\n' "$diagram" >&2
    exit 1
  fi
  if grep -Fq '/v1/apps/' "$diagram"; then
    printf 'Slack diagram still uses an App-scoped ingress route: %s\n' "$diagram" >&2
    exit 1
  fi
done

architecture_mmd="$ROOT/docs/images/integrations/slack-architecture.mmd"
architecture_svg="$ROOT/docs/images/integrations/slack-architecture.svg"
if ! grep -Fq 'Agent -->|runs on| Harness' "$architecture_mmd" ||
  ! grep -Fq '>runs on<' "$architecture_svg" ||
  grep -Fq '>inherits<' "$architecture_svg"; then
  printf 'Rendered Slack architecture does not match the Agent-to-Harness relationship.\n' >&2
  exit 1
fi
message_flow_mmd="$ROOT/docs/images/integrations/slack-message-flow.mmd"
message_flow_svg="$ROOT/docs/images/integrations/slack-message-flow.svg"
if ! grep -Fq 'D->>D: Unregister delivery' "$message_flow_mmd" ||
  ! grep -Fq 'Unregister delivery' "$message_flow_svg"; then
  printf 'Rendered Slack message flow omits delivery cleanup.\n' >&2
  exit 1
fi

apps_guide="$ROOT/docs/features/apps.md"
required_lifecycle_text=(
  'Draft ⇄ Live'
  'Draft → Disabled'
  'Live → Disabled'
  'Disabled → Draft'
)

for text in "${required_lifecycle_text[@]}"; do
  if ! grep -Fq "$text" "$apps_guide"; then
    printf 'Apps compatibility guide omits a channel lifecycle transition: %s\n' "$text" >&2
    exit 1
  fi
done

printf 'Public Slack documentation uses the Agent Integrations channel flow.\n'
