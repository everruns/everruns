#!/usr/bin/env bash

# This public, non-production key must stay stable so persisted local databases
# remain decryptable across restarts and worktrees.
DEFAULT_LOCAL_SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY="

configure_local_development_encryption_key() {
  if [ -z "${SECRETS_ENCRYPTION_KEY:-}" ]; then
    export SECRETS_ENCRYPTION_KEY="$DEFAULT_LOCAL_SECRETS_ENCRYPTION_KEY"
    return 0
  fi

  return 1
}
