#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot to GitHub Gist and add a PR comment
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Requires: GITHUB_TOKEN environment variable
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

# Base64 encode the image
SCREENSHOT_B64=$(base64 -w0 "$SCREENSHOT_PATH" 2>/dev/null || base64 "$SCREENSHOT_PATH")

# Create a gist with the base64 image
GIST_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  https://api.github.com/gists \
  -d "{
    \"description\": \"$DESCRIPTION\",
    \"public\": false,
    \"files\": {
      \"$FILENAME.b64\": {
        \"content\": \"$SCREENSHOT_B64\"
      },
      \"README.md\": {
        \"content\": \"# $DESCRIPTION\\n\\nThis gist contains a base64-encoded screenshot.\\n\\nTo view: decode the .b64 file or use the PR comment link.\"
      }
    }
  }")

GIST_ID=$(echo "$GIST_RESPONSE" | jq -r '.id // empty')
GIST_URL=$(echo "$GIST_RESPONSE" | jq -r '.html_url // empty')

if [ -z "$GIST_ID" ]; then
  echo "❌ Failed to create gist"
  echo "$GIST_RESPONSE" | jq .
  exit 1
fi

echo "✅ Gist created: $GIST_URL"

# Add comment to PR
echo "💬 Adding comment to PR #$PR_NUMBER..."

COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

Screenshot captured and uploaded to gist.

🔗 **[View Screenshot (base64)]($GIST_URL)**

To decode locally:
\`\`\`bash
curl -s $GIST_URL/raw/$FILENAME.b64 | base64 -d > $FILENAME
\`\`\`"

COMMENT_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  "https://api.github.com/repos/everruns/everruns/issues/$PR_NUMBER/comments" \
  -d "{\"body\": $(echo "$COMMENT_BODY" | jq -Rs .)}")

COMMENT_URL=$(echo "$COMMENT_RESPONSE" | jq -r '.html_url // empty')

if [ -z "$COMMENT_URL" ]; then
  echo "❌ Failed to add PR comment"
  echo "$COMMENT_RESPONSE" | jq .
  exit 1
fi

echo "✅ Comment added: $COMMENT_URL"
echo ""
echo "Done! Screenshot uploaded to gist and linked in PR comment."
