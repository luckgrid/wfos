#!/usr/bin/env python3
"""Ignore SIGTERM, then sleep. Used to prove second-signal SIGKILL escalation:
the first forwarded SIGTERM must not kill this process, so a second signal to
the controller must force-kill the group via SIGKILL instead.
"""
import signal
import sys
import time

signal.signal(signal.SIGTERM, signal.SIG_IGN)

marker = sys.argv[1]
delay = float(sys.argv[2]) if len(sys.argv) > 2 else 30.0

with open(marker, "w", encoding="utf-8") as f:
    f.write("ready")
    f.flush()

time.sleep(delay)
