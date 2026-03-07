#!/usr/bin/env bash
# Fetch sccache S3 secrets from Doppler and emit GITHUB_ENV lines.
# Usage in CI: ./scripts/ci-sccache-env.sh >> "$GITHUB_ENV"
set -euo pipefail

# Install Doppler CLI if not present
if ! command -v doppler &>/dev/null; then
  curl -sLf --retry 3 https://cli.doppler.com/install.sh | sudo sh >/dev/null 2>&1
fi

echo "SCCACHE_BUCKET=$(doppler secrets get SCCACHE_BUCKET --plain)"
echo "SCCACHE_REGION=$(doppler secrets get SCCACHE_REGION --plain)"
# SCCACHE_S3_KEY_PREFIX is optional; default to "sccache" if not set in Doppler
KEY_PREFIX=$(doppler secrets get SCCACHE_S3_KEY_PREFIX --plain 2>/dev/null || echo "sccache")
echo "SCCACHE_S3_KEY_PREFIX=${KEY_PREFIX}"
echo "AWS_ACCESS_KEY_ID=$(doppler secrets get SCCACHE_AWS_ACCESS_KEY_ID --plain)"
echo "AWS_SECRET_ACCESS_KEY=$(doppler secrets get SCCACHE_AWS_SECRET_ACCESS_KEY --plain)"
