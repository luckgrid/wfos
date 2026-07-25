#!/usr/bin/env python3
"""Exit with the integer code given as argv[1] (default 0)."""
import sys

code = int(sys.argv[1]) if len(sys.argv) > 1 else 0
sys.exit(code & 0xFF)
