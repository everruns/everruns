"""Create readable VHS pages from an actual captured run; never synthesize output."""
from pathlib import Path
import math
import re
import sys

ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
PAGE_LINES = 26
SECTION_LABELS = {"SUMMARY", "RELEASE CHECKS ON DISK", "CHANGELOG.md AFTER THE RUN"}


def visible(line):
    return ANSI.sub("", line)


def pages(transcript):
    groups = [[]]
    for line in transcript.splitlines():
        if visible(line).strip() in SECTION_LABELS and any(groups[-1]):
            groups.append([])
        groups[-1].append(line.rstrip())
    return [group[start:start + PAGE_LINES] for group in groups
            for start in range(0, len(group), PAGE_LINES)
            if any(group[start:start + PAGE_LINES])]


def render():
    root = Path(__file__).resolve().parent
    output = pages((root / "demo.txt").read_text())
    directory = root / ".demo-pages"
    directory.mkdir(exist_ok=True)
    tape = ["# Generated from demo.txt by render_demo.py.",
            "Output demo.gif", 'Set Shell "bash"', "Set Width 1440",
            "Set Height 800", "Set FontSize 18", 'Set Theme "Catppuccin Mocha"',
            "Set TypingSpeed 1ms"]
    for index, page in enumerate(output, 1):
        (directory / f"{index:02}.txt").write_text(
            f"RECORDED LIVE RUN  |  {index}/{len(output)}\n\n" + "\n".join(page) + "\n"
        )
        words = len(" ".join(visible(line) for line in page).split())
        seconds = min(14, max(7, math.ceil(words / 5)))
        tape += ["Hide", f'Type "clear; cat .demo-pages/{index:02}.txt"',
                 "Enter", "Sleep 300ms", "Show", f"Sleep {seconds}s"]
    (root / "demo.tape").write_text("\n".join(tape) + "\n")


if __name__ == "__main__":
    if "--check" in sys.argv:
        sample = "\x1b[1mREQUEST\x1b[0m\nline\n\x1b[1mSUMMARY\x1b[0m\nDone"
        result = pages(sample)
        assert len(result) == 2
        assert visible(result[1][0]) == "SUMMARY"
        assert sum("Done" in page for page in result) == 1
        assert all(len(page) <= PAGE_LINES for page in pages("\n".join(str(n) for n in range(100))))
        print("Recording pagination checks passed")
    else:
        render()
