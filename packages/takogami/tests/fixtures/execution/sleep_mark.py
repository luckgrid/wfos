#!/usr/bin/env python3
"""Sleep briefly, then append to a marker file (ordering probe)."""
import sys
import time
from pathlib import Path

delay = float(sys.argv[1]) if len(sys.argv) > 1 else 0.4
marker = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("MARKER_RAN")
time.sleep(delay)
with marker.open("a", encoding="utf-8") as f:
    f.write("ran\n")
