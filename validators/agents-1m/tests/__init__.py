import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIB = os.path.join(os.path.dirname(ROOT), "lib")
os.environ["VALIDATOR_ROOT"] = ROOT
if LIB not in sys.path:
    sys.path.insert(0, LIB)
