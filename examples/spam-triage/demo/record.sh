#!/usr/bin/env bash
# Record the demo GIF and transcript from a real run of the documented command.
#
# Two renderers, same command and same output files. VHS is the house style and
# is used whenever it is available. It draws frames through a headless browser,
# which will not launch as root in some containers; there the asciinema+agg
# path produces an equivalent GIF with no browser involved.
set -euo pipefail
demo_dir="$(cd "$(dirname "$0")" && pwd)"
example_dir="$(cd "$demo_dir/.." && pwd)"
repo_root="$(cd "$example_dir/../.." && pwd)"
tape="$demo_dir/demo.tape"
typed_run="$demo_dir/typed-run.sh"
session_tmp="$demo_dir/.session.tmp"
transcript_tmp="$demo_dir/.transcript.tmp"
cast_tmp="$demo_dir/.demo.tmp.cast"
gif_tmp="$demo_dir/.demo.tmp.gif"

# Catppuccin Mocha, the theme demo.tape selects, as agg's background,
# foreground, and sixteen ANSI colors.
AGG_THEME=1e1e2e,cdd6f4,45475a,f38ba8,a6e3a1,f9e2af,89b4fa,f5c2e7,94e2d5,bac2de,585b70,f38ba8,a6e3a1,f9e2af,89b4fa,f5c2e7,94e2d5,a6adc8

extract_transcript() {
  awk '
    /SCREEN / { capture = 1; sub(/^.*SCREEN /, "SCREEN ") }
    capture { gsub(/\r/, ""); print }
    /adjudication / { exit }
  ' "$1"
}

if [[ "${1:-}" == "--check" ]]; then
  command_line='cargo run -q -p everruns-spam-triage -- --limit 30'
  # Both renderers must run the same command, or the GIF and the tape drift.
  grep -Fq "$command_line" "$tape"
  grep -Fq "$command_line" "$typed_run"
  grep -Fq 'Wait+Screen /screening/' "$tape"
  grep -Fq 'Type@35ms' "$tape"
  grep -Fq 'Wait+Screen /SUMMARY/' "$tape"
  python3 -c "import ast, sys; ast.parse(open(sys.argv[1]).read())" "$demo_dir/sized-pty.py"
  sample="$(printf '\033[0mSCREEN jev\r\nSUMMARY\r\n  adjudication 1.00s\r\n> ' | extract_transcript /dev/stdin)"
  [[ "$sample" == $'SCREEN jev\nSUMMARY\n  adjudication 1.00s' ]]
  if command -v vhs >/dev/null 2>&1; then
    vhs validate "$tape"
  fi
  echo "Spam Triage demo checks passed"
  exit 0
fi

[[ -f "$example_dir/data/corpus.jsonl" ]] || {
  echo "build the corpus first: bash examples/spam-triage/data/build-corpus.sh" >&2
  exit 1
}

# Credentials reach the recorder through the environment. Re-exec under Doppler
# rather than wrapping the recorder, which is a shell function here.
if [[ -z "${TYPESAFE_API_KEY:-}" || -z "${OPENROUTER_API_KEY:-}" ]]; then
  if [[ -n "${SPAM_TRIAGE_RECORD_REENTERED:-}" ]]; then
    echo "Doppler supplied no TYPESAFE_API_KEY and OPENROUTER_API_KEY" >&2
    exit 1
  fi
  command -v doppler >/dev/null 2>&1 || {
    echo "set TYPESAFE_API_KEY and OPENROUTER_API_KEY or install Doppler to record the live demo" >&2
    exit 1
  }
  export SPAM_TRIAGE_RECORD_REENTERED=1
  exec doppler run --project everruns-dev --config dev -- "$0" "$@"
fi

record_with_vhs() {
  vhs "$tape"
}

record_without_browser() {
  python3 "$demo_dir/sized-pty.py" \
    asciinema rec --overwrite --idle-time-limit 2 -c "$typed_run" "$cast_tmp" \
    > "$session_tmp"
  agg --theme "$AGG_THEME" --font-size 15 --fps-cap 24 \
    --idle-time-limit 1.5 --last-frame-duration 5 "$cast_tmp" "$gif_tmp"
}

if command -v vhs >/dev/null 2>&1; then
  recorder=record_with_vhs
elif command -v asciinema >/dev/null 2>&1 && command -v agg >/dev/null 2>&1; then
  echo "vhs not found; recording with asciinema+agg"
  recorder=record_without_browser
else
  echo "install vhs, or asciinema and agg, to record the demo" >&2
  exit 1
fi

cd "$repo_root"
trap 'rm -f "$session_tmp" "$transcript_tmp" "$cast_tmp" "$gif_tmp"' EXIT
"$recorder"

extract_transcript "$session_tmp" > "$transcript_tmp"
grep -Fq 'adjudication' "$transcript_tmp"
mv "$transcript_tmp" "$demo_dir/transcript.txt"
mv "$gif_tmp" "$demo_dir/demo.gif"

echo "Recorded $demo_dir/demo.gif"
