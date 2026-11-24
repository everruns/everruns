"""The pages verifier's only hook: its check."""

import os
import re

from engine.checks import Check, Output
from engine.hooks import Hooks

SITE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "site")


def scan(ctx):
    pages = sorted(f for f in os.listdir(SITE) if f.endswith(".html"))
    langs = {}
    for name in pages:
        with open(os.path.join(SITE, name)) as f:
            m = re.search(r'<html[^>]*\blang="([^"]+)"', f.read())
        langs[name] = m.group(1) if m else None
    other = [n for n, lang in langs.items() if lang and lang != "uk"]
    out = Output()
    out.add("P1", "L1", not other, note="other languages: " + (", ".join(other) or "none"))
    out.add("P2", "L1", sum(lang == "uk" for lang in langs.values()) / max(len(pages), 1),
            note=f"{len(pages)} pages")
    return out


class PagesVerifier(Hooks):
    def checks(self):
        return [Check("scan", "L1", ["P1", "P2"], minutes=0, command="verify", run=scan)]


VERIFIER = PagesVerifier()
