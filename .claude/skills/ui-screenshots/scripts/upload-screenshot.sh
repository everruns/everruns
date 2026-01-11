#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the embedded image
#
# Uses GitHub Release assets for hosting (provides direct image URLs)
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN - Required for PR comments and release uploads
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

REPO="everruns/everruns"
RELEASE_TAG="screenshots"

# Generate unique filename with timestamp
ORIGINAL_FILENAME=$(basename "$SCREENSHOT_PATH")
EXTENSION="${ORIGINAL_FILENAME##*.}"
BASENAME="${ORIGINAL_FILENAME%.*}"
FILENAME="${BASENAME}-$(date +%Y%m%d-%H%M%S).${EXTENSION}"

echo "📤 Uploading screenshot: $FILENAME"

# Ensure screenshots release exists
echo "   Checking screenshots release..."
RELEASE_EXISTS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/$REPO/releases/tags/$RELEASE_TAG")

if [ "$RELEASE_EXISTS" != "200" ]; then
  echo "   Creating screenshots release..."
  curl -s -X POST \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/$REPO/releases" \
    -d "{
      \"tag_name\": \"$RELEASE_TAG\",
      \"name\": \"Screenshots\",
      \"body\": \"Storage for UI screenshots attached to PRs\",
      \"draft\": false,
      \"prerelease\": true
    }" > /dev/null
fi

# Get release upload URL
RELEASE_INFO=$(curl -s \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/$REPO/releases/tags/$RELEASE_TAG")

UPLOAD_URL=$(echo "$RELEASE_INFO" | jq -r '.upload_url' | sed 's/{.*}//')

# Upload the asset
echo "   Uploading to release..."
UPLOAD_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "Content-Type: application/octet-stream" \
  "${UPLOAD_URL}?name=${FILENAME}" \
  --data-binary "@$SCREENSHOT_PATH")

IMAGE_URL=$(echo "$UPLOAD_RESPONSE" | jq -r '.browser_download_url // empty')

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
  "https://api.github.com/repos/$REPO/issues/$PR_NUMBER/comments" \
  -d "{\"body\": $(echo "$COMMENT_BODY" | jq -Rs .)}")

COMMENT_URL=$(echo "$COMMENT_RESPONSE" | jq -r '.html_url // empty')

if [ -n "$COMMENT_URL" ]; then
  echo "✅ Comment added: $COMMENT_URL"
else
  echo "❌ Failed to add PR comment"
  echo "$COMMENT_RESPONSE" | jq .
  exit 1
fi
