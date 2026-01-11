#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the embedded image
#
# Uses Cloudinary for image hosting (signed upload)
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN   - Required for PR comments
#   CLOUDINARY_URL - Cloudinary URL: cloudinary://API_KEY:API_SECRET@CLOUD_NAME
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

if [ -z "${CLOUDINARY_URL:-}" ]; then
  echo "❌ CLOUDINARY_URL environment variable not set"
  echo "   Format: cloudinary://API_KEY:API_SECRET@CLOUD_NAME"
  exit 1
fi

# Parse CLOUDINARY_URL: cloudinary://API_KEY:API_SECRET@CLOUD_NAME
CLOUDINARY_URL_PARSED="${CLOUDINARY_URL#cloudinary://}"
API_KEY="${CLOUDINARY_URL_PARSED%%:*}"
REMAINDER="${CLOUDINARY_URL_PARSED#*:}"
API_SECRET="${REMAINDER%%@*}"
CLOUD_NAME="${REMAINDER#*@}"

if [ -z "$API_KEY" ] || [ -z "$API_SECRET" ] || [ -z "$CLOUD_NAME" ]; then
  echo "❌ Invalid CLOUDINARY_URL format"
  echo "   Expected: cloudinary://API_KEY:API_SECRET@CLOUD_NAME"
  exit 1
fi

FILENAME=$(basename "$SCREENSHOT_PATH")
TIMESTAMP=$(date +%s)

echo "📤 Uploading screenshot: $FILENAME"

# Generate signature for signed upload
# Parameters must be sorted alphabetically
PARAMS_TO_SIGN="folder=pr-screenshots&timestamp=${TIMESTAMP}"
SIGNATURE=$(echo -n "${PARAMS_TO_SIGN}${API_SECRET}" | sha1sum | cut -d' ' -f1)

# Upload to Cloudinary (signed upload)
echo "   Uploading to Cloudinary..."
UPLOAD_RESPONSE=$(curl -s -X POST \
  "https://api.cloudinary.com/v1_1/${CLOUD_NAME}/image/upload" \
  -F "file=@$SCREENSHOT_PATH" \
  -F "api_key=${API_KEY}" \
  -F "timestamp=${TIMESTAMP}" \
  -F "signature=${SIGNATURE}" \
  -F "folder=pr-screenshots")

IMAGE_URL=$(echo "$UPLOAD_RESPONSE" | jq -r '.secure_url // empty')

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
