#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the embedded image
#
# Uses freeimage.host for image hosting (provides direct image URLs)
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN - Required for PR comments
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

if [ -z "${GITHUB_TOKEN:-}" ]; then
  echo "❌ GITHUB_TOKEN environment variable not set"
  exit 1
fi

FILENAME=$(basename "$SCREENSHOT_PATH")
echo "📤 Uploading screenshot: $FILENAME"

# Upload to freeimage.host
echo "   Uploading to freeimage.host..."
UPLOAD_RESPONSE=$(curl -s -X POST "https://freeimage.host/api/1/upload" \
  -F "source=@$SCREENSHOT_PATH" \
  -F "key=6d207e02198a847aa98d0a2a901485a5" 2>/dev/null || echo '{"status_code":0}')

IMAGE_URL=$(echo "$UPLOAD_RESPONSE" | jq -r '.image.url // empty')

if [ -z "$IMAGE_URL" ]; then
  echo "❌ Upload failed"
  echo "$UPLOAD_RESPONSE" | jq .
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
  -H "Authorization: Bearer $GITHUB_TOKEN" \
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
