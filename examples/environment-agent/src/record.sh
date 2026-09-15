#!/usr/bin/env bash
# Record the demo from a real provider run. Never synthesize output.
set -euo pipefail
cd "$(dirname "$0")"

# Preserve the previous successful transcript if the provider call fails.
task_transcript=$(mktemp)
trap 'rm -f "$task_transcript"' EXIT
cargo run -q -p everruns-environment-agent -- "$@" | tee "$task_transcript"
mv "$task_transcript" demo.txt

python3 render_demo.py
rm -rf .demo-frames
vhs demo.tape

# VHS captures frames reliably but its own GIF output silently produces nothing
# in some environments, so the encode is explicit here rather than implied.
ffmpeg -y -loglevel error -framerate 50 -i .demo-frames/frame-text-%05d.png \
  -vf "fps=12,scale=1200:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3" \
  -loop 0 demo.gif
echo "wrote $(pwd)/demo.gif"
