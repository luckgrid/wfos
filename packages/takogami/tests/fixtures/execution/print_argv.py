#!/usr/bin/env python3
"""Report argv as length-prefixed JSON (proves literal token boundaries)."""
import json
import sys

doc = {
    "argv": [{"len": len(a.encode("utf-8")), "value": a} for a in sys.argv[1:]],
}
sys.stdout.write(json.dumps(doc, ensure_ascii=False, separators=(",", ":")))
sys.stdout.write("\n")
