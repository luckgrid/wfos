#!/usr/bin/env python3
"""Sleep with default (terminate-by-signal) disposition for SIGINT/SIGTERM/SIGHUP.

Used to prove real signal forwarding: the controller must forward the exact
received signal to this process's group, and the OS default action must kill
it via that signal (WIFSIGNALED), not via a caught Python exception.
"""
import signal
import sys
import time

signal.signal(signal.SIGINT, signal.SIG_DFL)
signal.signal(signal.SIGTERM, signal.SIG_DFL)
signal.signal(signal.SIGHUP, signal.SIG_DFL)

marker = sys.argv[1]
delay = float(sys.argv[2]) if len(sys.argv) > 2 else 30.0

with open(marker, "w", encoding="utf-8") as f:
    f.write("ready")
    f.flush()

time.sleep(delay)
