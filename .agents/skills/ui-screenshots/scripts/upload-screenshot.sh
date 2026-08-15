#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the embedded image
#
# Uses GitHub's user-attachments CDN, the same storage as drag-and-drop uploads.
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN - Optional when `gh auth token` is available; must have push access
#
# Example:
#   ./upload-screenshot.sh screenshot.png 195 "Dev components page"

SCREENSHOT_PATH="${1:-}"
PR_NUMBER="${2:-}"
DESCRIPTION="${3:-UI Screenshot}"

if [ -z "$SCREENSHOT_PATH" ] || [ -z "$PR_NUMBER" ]; then
  echo "Usage: $0 <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]"
  exit 1
fi

if [ ! -f "$SCREENSHOT_PATH" ]; then
  echo "❌ Screenshot not found: $SCREENSHOT_PATH"
  exit 1
fi

if [ -n "${GITHUB_TOKEN:-}" ]; then
  TOKEN="$GITHUB_TOKEN"
elif command -v gh &> /dev/null && TOKEN=$(gh auth token 2>/dev/null); then
  :
else
  echo "❌ Set GITHUB_TOKEN or authenticate the gh CLI"
  exit 1
fi

FILENAME=$(basename "$SCREENSHOT_PATH")
CONTENT_TYPE=$(file -b --mime-type "$SCREENSHOT_PATH")

echo "📤 Uploading screenshot: $FILENAME"

REPOSITORY_RESPONSE=$(curl -sS \
  -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  "https://api.github.com/repos/everruns/everruns")
REPOSITORY_ID=$(echo "$REPOSITORY_RESPONSE" | jq -r '.id // empty')

if [ -z "$REPOSITORY_ID" ]; then
  echo "❌ Failed to resolve the everruns/everruns repository ID"
  echo "$REPOSITORY_RESPONSE" | jq .
  exit 1
fi

ENCODED_FILENAME=$(jq -rn --arg value "$FILENAME" '$value | @uri')
ENCODED_CONTENT_TYPE=$(jq -rn --arg value "$CONTENT_TYPE" '$value | @uri')
UPLOAD_URL="https://uploads.github.com/user-attachments/assets?name=${ENCODED_FILENAME}&content_type=${ENCODED_CONTENT_TYPE}&repository_id=${REPOSITORY_ID}"
UPLOAD_RESPONSE=$(curl -sS \
  -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/json" \
  --data-binary "@$SCREENSHOT_PATH" \
  "$UPLOAD_URL")

IMAGE_URL=$(echo "$UPLOAD_RESPONSE" | jq -r '.url // empty')

if [ -z "$IMAGE_URL" ]; then
  echo "❌ Upload failed"
  echo "$UPLOAD_RESPONSE" | jq .
  echo "   HTTP 422 means unsupported media; HTTP 404 usually means a bad repository ID or no push access."
  exit 1
fi

echo "   ✅ Uploaded: $IMAGE_URL"

# Build comment body with embedded image
echo "💬 Adding comment to PR #$PR_NUMBER..."

COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

![$DESCRIPTION]($IMAGE_URL)"

# Post comment
COMMENT_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  "https://api.github.com/repos/everruns/everruns/issues/$PR_NUMBER/comments" \
  -d "{\"body\": $(echo "$COMMENT_BODY" | jq -Rs .)}")

COMMENT_URL=$(echo "$COMMENT_RESPONSE" | jq -r '.html_url // empty')

if [ -n "$COMMENT_URL" ]; then
  echo "✅ Comment added: $COMMENT_URL"
else
  echo "❌ Failed to add PR comment"
  echo "$COMMENT_RESPONSE" | jq .
  exit 1
fi
