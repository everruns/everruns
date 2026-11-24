import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIB = os.path.join(os.path.dirname(ROOT), "lib")
os.environ["VERIFIER_ROOT"] = ROOT
for p in (ROOT, LIB):  # the verifier (its checks, workload) and the engine
    if p not in sys.path:
        sys.path.insert(0, p)
