#!/usr/bin/env bash
# Offline checks for the CI tool installers that avoid the Ubuntu mirror.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. An rg already on PATH is used as is: no download, no apt.
mkdir -p "$tmp/present"
cat > "$tmp/present/rg" <<'SH'
#!/usr/bin/env bash
echo "ripgrep 99.0.0"
SH
for tool in curl sudo apt-get; do
  printf '#!/usr/bin/env bash\necho "unexpected %s" >&2; exit 97\n' "$tool" > "$tmp/present/$tool"
done
chmod +x "$tmp/present/"*
out="$(PATH="$tmp/present:$PATH" bash "$ROOT/scripts/ci-install-ripgrep.sh")"
[ "$out" = "ripgrep 99.0.0" ] || fail "existing rg not reused: $out"

# The remaining cases need the pinned-binary branch.
case "$(uname -m)" in
  x86_64) target="x86_64-unknown-linux-musl" ;;
  aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
  *) echo "ok (pinned-binary cases skipped on $(uname -m))"; exit 0 ;;
esac

# A PATH without rg: keep only the tools the installer needs.
mkdir -p "$tmp/bin"
for tool in bash curl sha256sum tar gzip mktemp install rm head uname; do
  ln -s "$(command -v "$tool")" "$tmp/bin/$tool"
done

# 2. A tarball whose checksum does not match the pin is refused.
dir="ripgrep-14.1.1-${target}"
mkdir -p "$tmp/release/$dir"
printf '#!/usr/bin/env bash\necho "ripgrep tampered"\n' > "$tmp/release/$dir/rg"
chmod +x "$tmp/release/$dir/rg"
tar -czf "$tmp/release/$dir.tar.gz" -C "$tmp/release" "$dir/rg"
mkdir -p "$tmp/install"
if PATH="$tmp/bin" RG_BASE_URL="file://$tmp/release" RG_INSTALL_DIR="$tmp/install" \
  bash "$ROOT/scripts/ci-install-ripgrep.sh" >"$tmp/out" 2>&1; then
  fail "checksum mismatch was accepted"
fi
[ ! -e "$tmp/install/rg" ] || fail "tampered rg was installed"

echo "ok"
