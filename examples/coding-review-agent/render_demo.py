"""Create readable VHS pages from an actual captured run; never synthesize output."""
from pathlib import Path
import math
import re
import sys
import textwrap

def pages(transcript):
    lines = []
    in_code = False
    for line in transcript.splitlines():
        if line.lstrip().startswith("```"):
            in_code = not in_code
            continue
        if not in_code:
            line = re.sub(r"^#{1,6} ", "", line).replace("**", "").replace("`", "")
        indent = line[:len(line) - len(line.lstrip())]
        lines.extend(textwrap.wrap(line, width=94, replace_whitespace=False,
                     subsequent_indent=indent, break_long_words=False,
                     break_on_hyphens=False) or [""])
    # Start the answer on its own page; never repeat the last screen to fill time.
    sections = "\n".join(lines).split("\nANSWER\n", 1)
    groups = [sections[0].splitlines()]
    if len(sections) == 2:
        groups.append(["ANSWER", ""] + sections[1].strip("\n").splitlines())
    return [group[start:start + 26] for group in groups
            for start in range(0, len(group), 26) if any(group[start:start + 26])]

def render():
    root = Path(__file__).resolve().parent
    output = pages((root / "demo.txt").read_text())
    directory = root / ".demo-pages"
    directory.mkdir(exist_ok=True)
    tape = ['# Generated from demo.txt by render_demo.py.',
            'Output demo.gif', 'Set Shell "bash"', 'Set Width 1280',
            'Set Height 800', 'Set FontSize 19', 'Set Theme "Catppuccin Mocha"',
            'Set TypingSpeed 1ms']
    for index, page in enumerate(output, 1):
        (directory / f"{index:02}.txt").write_text(
            f"RECORDED LIVE RUN  |  {index}/{len(output)}\n\n" + "\n".join(page) + "\n")
        seconds = min(25, max(8, math.ceil(len(" ".join(page).split()) / 3)))
        tape += ['Hide', f'Type "clear; cat .demo-pages/{index:02}.txt"',
                 'Enter', 'Sleep 300ms', 'Show', f'Sleep {seconds}s']
    (root / "demo.tape").write_text("\n".join(tape) + "\n")

if __name__ == "__main__":
    if "--check" in sys.argv:
        sample = "QUESTION\nTest\n\nANSWER\n\n```rust\n    let x = 1;\n```\nDone"
        result = pages(sample)
        assert any("    let x = 1;" in page for page in result)
        assert len(result) == 2
        assert sum("Done" in page for page in result) == 1
        assert all(len(page) <= 26 for page in pages("\n".join(str(n) for n in range(100))))
        print("Recording pagination checks passed")
    else:
        render()
