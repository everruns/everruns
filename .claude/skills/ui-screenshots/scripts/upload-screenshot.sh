#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the image
#
# Upload methods (in order of preference):
# 1. GitHub Gist (using GITHUB_GIST_TOKEN or GITHUB_TOKEN)
# 2. Imgur (anonymous upload, no API key needed)
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN      - Required for PR comments (can be org-scoped fine-grained PAT)
#   GITHUB_GIST_TOKEN - Optional, for gist uploads (classic token with 'gist' scope)
#                       If not set, falls back to GITHUB_TOKEN
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

IMAGE_URL=""

# Use GITHUB_GIST_TOKEN if available, otherwise fall back to GITHUB_TOKEN
GIST_TOKEN="${GITHUB_GIST_TOKEN:-$GITHUB_TOKEN}"

# Method 1: Try GitHub Gist
echo "   Trying GitHub Gist..."
SCREENSHOT_B64=$(base64 -w0 "$SCREENSHOT_PATH" 2>/dev/null || base64 "$SCREENSHOT_PATH")

GIST_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $GIST_TOKEN" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  https://api.github.com/gists \
  -d "{
    \"description\": \"$DESCRIPTION\",
    \"public\": false,
    \"files\": {
      \"$FILENAME.b64\": {
        \"content\": \"$SCREENSHOT_B64\"
      }
    }
  }" 2>/dev/null || echo '{"message":"failed"}')

GIST_ID=$(echo "$GIST_RESPONSE" | jq -r '.id // empty')

if [ -n "$GIST_ID" ]; then
  echo "   ✅ Gist created: https://gist.github.com/$GIST_ID"
  IMAGE_URL="gist:$GIST_ID"
fi

# Method 2: Try Imgur (anonymous upload)
if [ -z "$IMAGE_URL" ]; then
  echo "   Gist failed, trying Imgur..."

  IMGUR_RESPONSE=$(curl -s -X POST \
    -H "Authorization: Client-ID 546c25a59c58ad7" \
    -F "image=@$SCREENSHOT_PATH" \
    https://api.imgur.com/3/image 2>/dev/null || echo '{"success":false}')

  IMGUR_URL=$(echo "$IMGUR_RESPONSE" | jq -r '.data.link // empty')

  if [ -n "$IMGUR_URL" ]; then
    echo "   ✅ Imgur upload successful"
    IMAGE_URL="$IMGUR_URL"
  fi
fi

# Build comment body
echo "💬 Adding comment to PR #$PR_NUMBER..."

if [[ "$IMAGE_URL" == gist:* ]]; then
  GIST_ID="${IMAGE_URL#gist:}"
  COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

🔗 **[View Screenshot (base64 encoded)](https://gist.github.com/$GIST_ID)**

<details>
<summary>Decode instructions</summary>

\`\`\`bash
curl -sL https://gist.github.com/$GIST_ID/raw/$FILENAME.b64 | base64 -d > $FILENAME
\`\`\`
</details>"

elif [ -n "$IMAGE_URL" ]; then
  COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

![$DESCRIPTION]($IMAGE_URL)"

else
  echo "❌ All upload methods failed"
  COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

⚠️ Image upload failed. Screenshot available locally at:
\`$SCREENSHOT_PATH\`

To reproduce:
\`\`\`bash
./scripts/dev.sh e2e-screenshots
\`\`\`"
fi

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
