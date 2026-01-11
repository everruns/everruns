#!/usr/bin/env bash
set -euo pipefail

# Upload a screenshot and add a PR comment with the embedded image
#
# Upload methods (in order of preference):
# 1. GitHub Gist with HTML preview (using GITHUB_GIST_TOKEN)
# 2. catbox.moe (anonymous, direct image URL)
# 3. Imgur (anonymous upload)
#
# Usage: upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
#
# Environment variables:
#   GITHUB_TOKEN      - Required for PR comments
#   GITHUB_GIST_TOKEN - Optional, for gist uploads (classic token with 'gist' scope)
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
GIST_TOKEN="${GITHUB_GIST_TOKEN:-$GITHUB_TOKEN}"

# Method 1: Try GitHub Gist with HTML that embeds the image
if [ -n "$GIST_TOKEN" ]; then
  echo "   Trying GitHub Gist (HTML preview)..."
  SCREENSHOT_B64=$(base64 -w0 "$SCREENSHOT_PATH" 2>/dev/null || base64 "$SCREENSHOT_PATH")

  # Detect mime type
  MIME_TYPE="image/png"
  if [[ "$FILENAME" == *.jpg ]] || [[ "$FILENAME" == *.jpeg ]]; then
    MIME_TYPE="image/jpeg"
  fi

  # Create HTML file with embedded image
  HTML_CONTENT="<!DOCTYPE html>
<html>
<head><title>$DESCRIPTION</title></head>
<body style=\"margin:0;padding:20px;background:#1a1a1a;\">
<h2 style=\"color:#fff;font-family:system-ui;\">$DESCRIPTION</h2>
<img src=\"data:$MIME_TYPE;base64,$SCREENSHOT_B64\" style=\"max-width:100%;border:1px solid #333;\">
</body>
</html>"

  # Create gist with HTML file
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

  if [ -n "$GIST_ID" ] && [ -n "$GIST_OWNER" ]; then
    echo "   ✅ Gist created: https://gist.github.com/$GIST_ID"
    # Use bl.ocks.org or gist.github.com raw URL for HTML preview
    IMAGE_URL="gist:$GIST_OWNER:$GIST_ID"
  fi
fi

# Method 2: Try catbox.moe (anonymous, direct URL)
if [ -z "$IMAGE_URL" ]; then
  echo "   Trying catbox.moe..."
  CATBOX_RESPONSE=$(curl -s -X POST \
    -F "reqtype=fileupload" \
    -F "fileToUpload=@$SCREENSHOT_PATH" \
    https://catbox.moe/user/api.php 2>/dev/null || echo "")

  if [[ "$CATBOX_RESPONSE" == https://* ]]; then
    echo "   ✅ catbox.moe upload successful"
    IMAGE_URL="$CATBOX_RESPONSE"
  fi
fi

# Method 3: Try Imgur (anonymous upload)
if [ -z "$IMAGE_URL" ]; then
  echo "   catbox.moe failed, trying Imgur..."

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
  # Parse gist info: gist:owner:id
  GIST_INFO="${IMAGE_URL#gist:}"
  GIST_OWNER="${GIST_INFO%%:*}"
  GIST_ID="${GIST_INFO#*:}"

  # Raw URL for the HTML file (renders in browser)
  RAW_URL="https://gist.githubusercontent.com/$GIST_OWNER/$GIST_ID/raw/screenshot.html"

  COMMENT_BODY="## 📸 UI Screenshot

**$DESCRIPTION**

🔗 **[View Screenshot]($RAW_URL)** (click to open)

<sub>Gist: https://gist.github.com/$GIST_ID</sub>"

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
