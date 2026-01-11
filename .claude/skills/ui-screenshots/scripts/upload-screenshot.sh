#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the image
#
# Uses GitHub Gist with HTML preview (renders image when opened)
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN      - Required for PR comments
#   GITHUB_GIST_TOKEN - Required for gist uploads (classic token with 'gist' scope)
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

GIST_TOKEN="${GITHUB_GIST_TOKEN:-$GITHUB_TOKEN}"

FILENAME=$(basename "$SCREENSHOT_PATH")
echo "📤 Uploading screenshot: $FILENAME"

# Create HTML with embedded image
echo "   Creating gist with HTML preview..."
SCREENSHOT_B64=$(base64 -w0 "$SCREENSHOT_PATH" 2>/dev/null || base64 "$SCREENSHOT_PATH")

MIME_TYPE="image/png"
if [[ "$FILENAME" == *.jpg ]] || [[ "$FILENAME" == *.jpeg ]]; then
  MIME_TYPE="image/jpeg"
fi

HTML_CONTENT="<!DOCTYPE html>
<html>
<head><title>$DESCRIPTION</title></head>
<body style=\"margin:0;padding:20px;background:#1a1a1a;\">
<h2 style=\"color:#fff;font-family:system-ui;\">$DESCRIPTION</h2>
<img src=\"data:$MIME_TYPE;base64,$SCREENSHOT_B64\" style=\"max-width:100%;border:1px solid #333;\">
</body>
</html>"

GIST_PAYLOAD=$(jq -n \
  --arg desc "$DESCRIPTION" \
  --arg filename "screenshot.html" \
  --arg content "$HTML_CONTENT" \
  '{description: $desc, public: false, files: {($filename): {content: $content}}}')

GIST_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $GIST_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  https://api.github.com/gists \
  -d "$GIST_PAYLOAD" 2>/dev/null || echo '{"message":"failed"}')

GIST_ID=$(echo "$GIST_RESPONSE" | jq -r '.id // empty')
GIST_OWNER=$(echo "$GIST_RESPONSE" | jq -r '.owner.login // empty')

if [ -z "$GIST_ID" ] || [ -z "$GIST_OWNER" ]; then
  echo "❌ Gist upload failed"
  echo "$GIST_RESPONSE" | jq .
  exit 1
fi

echo "   ✅ Gist created: https://gist.github.com/$GIST_ID"

# Raw URL renders the HTML with embedded image
RAW_URL="https://gist.githubusercontent.com/$GIST_OWNER/$GIST_ID/raw/screenshot.html"

# Build comment body
echo "💬 Adding comment to PR #$PR_NUMBER..."

COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

🔗 **[View Screenshot]($RAW_URL)** (click to open)

<sub>Gist: https://gist.github.com/$GIST_ID</sub>"

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
