#!/usr/bin/env bash
# Check if cloud agent is configured for UI screenshots

MISSING=()

echo "🔍 Checking UI Screenshots configuration..."
echo ""

# Check GITHUB_TOKEN
if [ -z "${GITHUB_TOKEN:-}" ]; then
  MISSING+=("GITHUB_TOKEN")
  echo "❌ GITHUB_TOKEN - not set"
else
  echo "✅ GITHUB_TOKEN - set"
fi

# Check CLOUDINARY_URL
if [ -z "${CLOUDINARY_URL:-}" ]; then
  MISSING+=("CLOUDINARY_URL")
  echo "❌ CLOUDINARY_URL - not set"
else
  # Validate format
  if [[ "$CLOUDINARY_URL" =~ ^cloudinary://[^:]+:[^@]+@.+$ ]]; then
    CLOUD_NAME="${CLOUDINARY_URL##*@}"
    echo "✅ CLOUDINARY_URL - set (cloud: $CLOUD_NAME)"
  else
    MISSING+=("CLOUDINARY_URL (invalid format)")
    echo "❌ CLOUDINARY_URL - invalid format (expected: cloudinary://API_KEY:API_SECRET@CLOUD_NAME)"
  fi
fi

# Check agent-browser
if command -v agent-browser &> /dev/null; then
  AGENT_BROWSER_VERSION=$(agent-browser --version 2>/dev/null || echo "unknown")
  echo "✅ agent-browser - installed ($AGENT_BROWSER_VERSION)"
else
  MISSING+=("agent-browser")
  echo "❌ agent-browser - not installed"
  echo "   Install with: npm install -g agent-browser && agent-browser install"
fi

echo ""

if [ ${#MISSING[@]} -eq 0 ]; then
  echo "✅ Cloud agent is configured for UI screenshots"
  exit 0
else
  echo "❌ Missing configuration:"
  for item in "${MISSING[@]}"; do
    echo "   - $item"
  done
  echo ""
  echo "Add missing secrets to cloud agent environment."
  exit 1
fi
