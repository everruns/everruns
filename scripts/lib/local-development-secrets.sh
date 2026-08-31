#!/usr/bin/env bash

# Kept only to decrypt records written by older local stacks. New records use a
# private, per-prefix key stored outside source control.
LEGACY_LOCAL_SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY="

configure_local_development_encryption_key() {
  if [ -n "${SECRETS_ENCRYPTION_KEY:-}" ]; then
    return 1
  fi

  local project_root secrets_dir key_file temporary_key prefix
  project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  secrets_dir="${LOCAL_DEVELOPMENT_SECRETS_DIR:-$project_root/.local/agent-dev}"
  prefix="${PORT_PREFIX:-default}"
  key_file="$secrets_dir/secrets-encryption-key-$prefix"

  mkdir -p "$secrets_dir"
  chmod 700 "$secrets_dir"
  if [ ! -s "$key_file" ]; then
    temporary_key="$(mktemp "$secrets_dir/.secrets-encryption-key.XXXXXX")"
    chmod 600 "$temporary_key"
    python3 -c \
      'import base64, os; print("kek-local-v1:" + base64.b64encode(os.urandom(32)).decode())' \
      > "$temporary_key"
    # Another concurrent launcher may have won; never replace its established key.
    ln "$temporary_key" "$key_file" 2>/dev/null || true
    rm -f "$temporary_key"
  fi
  chmod 600 "$key_file"

  export SECRETS_ENCRYPTION_KEY="$(<"$key_file")"
  if [ -z "${SECRETS_ENCRYPTION_KEY_PREVIOUS:-}" ]; then
    export SECRETS_ENCRYPTION_KEY_PREVIOUS="$LEGACY_LOCAL_SECRETS_ENCRYPTION_KEY"
  fi
  return 0
}
