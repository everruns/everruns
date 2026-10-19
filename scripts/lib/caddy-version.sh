#!/usr/bin/env bash
# Older Caddy closes unread POST bodies after a failed dial, breaking safe retries.
# https://github.com/caddyserver/caddy/issues/7713
caddy_supports_connect_retries() {
  local version
  version="$(caddy version 2>/dev/null)" || return 1
  if [[ "$version" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)([[:space:]]|$) ]]; then
    local major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"
    (( major > 2 || (major == 2 && (minor > 11 || (minor == 11 && patch >= 4))) ))
  else
    return 1
  fi
}

require_caddy_connect_retries() {
  if ! caddy_supports_connect_retries; then
    echo "Caddy 2.11.4 or newer is required for safe POST connection retries. Upgrade Caddy or run: just init" >&2
    return 1
  fi
}
