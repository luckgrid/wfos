#!/usr/bin/env python3
"""Spawn a grandchild that stays in the same OS process group, record its PID,
then sleep. Used to prove process-group-wide signal delivery reaches
descendants, not only the direct child.
"""
import signal
import subprocess
import sys
import time

signal.signal(signal.SIGTERM, signal.SIG_DFL)
signal.signal(signal.SIGINT, signal.SIG_DFL)

marker = sys.argv[1]
delay = float(sys.argv[2]) if len(sys.argv) > 2 else 30.0

grandchild = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
with open(marker, "w", encoding="utf-8") as f:
    f.write(str(grandchild.pid))
    f.flush()

time.sleep(delay)
