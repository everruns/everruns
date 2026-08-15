#!/usr/bin/env bash
# Check if the environment is configured for UI screenshots

MISSING=()

echo "🔍 Checking UI Screenshots configuration..."
echo ""

# Check GitHub authentication
if [ -z "${GITHUB_TOKEN:-}" ]; then
  if command -v gh &> /dev/null && gh auth token &> /dev/null; then
    echo "✅ GitHub authentication - gh auth token"
  else
    MISSING+=("GITHUB_TOKEN or authenticated gh CLI")
    echo "❌ GitHub authentication - unavailable"
  fi
else
  echo "✅ GitHub authentication - GITHUB_TOKEN"
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
  echo "✅ Environment is configured for UI screenshots"
  exit 0
else
  echo "❌ Missing configuration:"
  for item in "${MISSING[@]}"; do
    echo "   - $item"
  done
  echo ""
  echo "Configure the missing dependency or credential."
  exit 1
fi
