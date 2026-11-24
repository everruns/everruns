"""L1 formal: the data plane v2 protocol models (tests/models/data-plane-v2)
checked with TLC by scripts/check-tla-models.sh.

Every model has a safe configuration that must pass and bug configurations
that must still fail (a bug configuration that stops failing means the model
no longer checks what it claims). The point passes when the script does.
Needs java; the script downloads tla2tools.jar once (or TLA2TOOLS_JAR).
"""

import os
import re
import subprocess

from engine.config import REPO
from . import Check, Output

SCRIPT = os.path.join(REPO, "scripts", "check-tla-models.sh")


def parse(text, code):
    oks = re.findall(r"^ok\s+(\S+)", text, re.M)
    fails = re.findall(r"^FAIL\s+(\S+?):", text, re.M)
    out = Output()
    if not oks and not fails:
        out.add("U1", "L1", False, note="the model check printed no results (TLC missing or download blocked?)")
        return out
    note = f"{len(oks)} configurations as expected" + (f", wrong: {', '.join(fails)}" if fails else "")
    out.add("U1", "L1", code == 0 and not fails, note=note)
    return out


def run(ctx):
    r = subprocess.run(["bash", SCRIPT], cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if ctx.evidence:
        with open(os.path.join(ctx.evidence, "tla-models.log"), "w") as f:
            f.write(r.stdout)
    out = parse(r.stdout, r.returncode)
    for e in out.entries:
        e["evidence"] = "scripts/check-tla-models.sh"
    return out


CHECKS = [Check("tla-models", "L1", ["U1"], minutes=8, needs=("java",), run=run,
                command="scripts/check-tla-models.sh")]
